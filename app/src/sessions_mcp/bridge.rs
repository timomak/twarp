//! twarp 26b/26c: the `twarp-sessions` MCP bridge, runtime, and
//! per-connection service — the `browser_mcp.rs` shape. 26b's read surface
//! (`list_sessions`, `get_transcript`, `list_projects`) plus 26c's event
//! surface (`watch_session`, `wait_for_completion`). Session create lands in
//! 26d.

use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::{mpsc, Mutex as StdMutex},
    time::{Duration, SystemTime},
};

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{
        CallToolResult, Content, Implementation, LoggingLevel, LoggingMessageNotificationParam,
        ServerCapabilities, ServerInfo,
    },
    schemars, tool, tool_handler, tool_router, ErrorData as McpError, Peer, RoleServer,
    ServerHandler,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio_util::sync::CancellationToken;
use twarpui::{Entity, ModelContext, ModelSpawner, SingletonEntity};

use super::{
    events::{self, WaitOutcome, WatchNotification},
    external::{self, ExternalListenerState},
    registry::{items_since, SessionRegistry},
    spawn::{SpawnParent, SpawnRefusal},
    status::SessionStatus,
    store,
};
use crate::settings::AgentSettings;
use crate::workspace::WorkspaceRegistry;
use ::settings::Setting as _;
use twarpui::GetSingletonModelHandle as _;

const SERVER_NAME: &str = "twarp-sessions";
const SSE_PATH: &str = "/sse";
const POST_PATH: &str = "/message";

pub(crate) struct SessionsMcpBridge {
    server: Option<SessionsMcpRuntime>,
    /// 26d: current state of the external token-gated listener, for the
    /// settings page (PRODUCT P#30: a port conflict is an explicit error
    /// state, never silent).
    external_state: ExternalListenerState,
}

struct SessionsMcpRuntime {
    url: String,
    spawner: ModelSpawner<SessionsMcpBridge>,
    /// One extra SSE server per Claude session (keyed by session id), so
    /// every tool call carries the calling session's identity — the same
    /// scoping the other built-in servers use.
    session_servers: StdMutex<HashMap<String, String>>,
    /// 26d: the external token-gated listener, when enabled (PRODUCT P#21).
    /// Cancelling the token stops the listener and severs its live watchers.
    external: StdMutex<Option<ExternalListener>>,
    runtime: tokio::runtime::Runtime,
    cancel: CancellationToken,
}

struct ExternalListener {
    cancel: CancellationToken,
    port: u16,
}

impl SessionsMcpBridge {
    pub(crate) fn new(ctx: &mut ModelContext<Self>) -> Self {
        let spawner = ctx.spawner();
        let server = SessionsMcpRuntime::start(spawner)
            .inspect_err(|err| log::warn!("Failed to start twarp sessions MCP server: {err}"))
            .ok();

        let mut bridge = Self {
            server,
            external_state: ExternalListenerState::Disabled,
        };
        // 26d: bring the external listener up per the persisted settings and
        // track future settings changes (toggle / port edits on the Agents
        // settings page).
        bridge.apply_external_listener_settings(ctx);
        let settings = ctx.get_singleton_model_handle::<AgentSettings>();
        ctx.observe(&settings, |bridge, _settings, ctx| {
            bridge.apply_external_listener_settings(ctx);
        });
        bridge
    }

    /// 26d: reconcile the external listener with the settings (PRODUCT P#21,
    /// 30). Enabling generates the bearer-token file and binds the fixed
    /// port; a bind failure becomes a settings-page error state. Disabling
    /// (or a port change) cancels the listener's token, which severs its live
    /// watchers (P#12).
    fn apply_external_listener_settings(&mut self, ctx: &mut ModelContext<Self>) {
        let settings = AgentSettings::as_ref(ctx);
        let enabled = *settings.sessions_external_enabled.value();
        let port =
            u16::try_from(*settings.sessions_external_port.value()).unwrap_or(0);
        let Some(runtime) = self.server.as_ref() else {
            return;
        };
        let desired = enabled.then_some(port);
        let current = runtime.external_port();
        if current == desired && !matches!(self.external_state, ExternalListenerState::Failed { .. })
        {
            return;
        }
        runtime.stop_external();
        self.external_state = match desired {
            None => ExternalListenerState::Disabled,
            Some(0) => ExternalListenerState::Failed {
                port,
                error: "Invalid port".to_owned(),
            },
            Some(port) => match external::ensure_token(&external::token_file_path())
                .map_err(|err| err.to_string())
                .and_then(|_| runtime.start_external(port))
            {
                Ok(()) => ExternalListenerState::Running { port },
                Err(error) => {
                    log::warn!(
                        "Failed to start external sessions MCP listener on port {port}: {error}"
                    );
                    ExternalListenerState::Failed { port, error }
                }
            },
        };
        ctx.notify();
    }

    /// 26d: the external listener's state, for the settings page.
    pub(crate) fn external_listener_state(&self) -> &ExternalListenerState {
        &self.external_state
    }

    /// 26d: mint a fresh external bearer token. The middleware re-reads the
    /// token file per request, so the old token is rejected immediately
    /// (PRODUCT P#21).
    pub(crate) fn regenerate_external_token(&self) -> Result<(), String> {
        external::regenerate_token(&external::token_file_path())
            .map(|_| ())
            .map_err(|err| err.to_string())
    }

    /// Per-session MCP config (PRODUCT P#19: auto-injected into every agent
    /// session alongside the other built-in servers). Falls back to the
    /// shared unscoped endpoint if the scoped server can't start.
    pub(crate) fn mcp_config_json_for_session(&self, session_id: &str) -> Option<String> {
        let runtime = self.server.as_ref()?;
        let url = runtime
            .session_server_url(session_id)
            .unwrap_or_else(|err| {
                log::warn!("Failed to start session-scoped sessions MCP server: {err}");
                runtime.url.clone()
            });
        Some(
            json!({
                "mcpServers": {
                    SERVER_NAME: {
                        "type": "sse",
                        "url": url,
                    }
                }
            })
            .to_string(),
        )
    }

    /// Main-thread read of the sidebar projects for `list_projects`
    /// (PRODUCT P#17). Projects carry no persisted color (color is a per-tab
    /// property), so `color` is null; `session_count` is the stored-session
    /// count for the project's directory across both providers.
    fn list_projects(&self, ctx: &ModelContext<Self>) -> Vec<serde_json::Value> {
        crate::projects::ProjectManagementModel::as_ref(ctx)
            .all_projects()
            .map(|project| {
                let path = PathBuf::from(&project.path);
                let name = project
                    .name
                    .clone()
                    .filter(|name| !name.trim().is_empty())
                    .or_else(|| {
                        path.file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                    })
                    .unwrap_or_else(|| project.path.clone());
                json!({
                    "name": name,
                    "color": serde_json::Value::Null,
                    "cwd": project.path,
                    "session_count": claude_code::sessions::list_sessions(&path).len(),
                })
            })
            .collect()
    }
}

/// 26d: a `create_chat` / `create_project` failure, mapped to the distinct
/// structured MCP errors of PRODUCT P#27.
#[derive(Debug)]
enum SpawnToolError {
    InvalidArgument(String),
    AtCapacity { limit: usize },
    DepthExceeded { depth: u8, max: u8 },
    Internal(String),
}

impl From<SpawnRefusal> for SpawnToolError {
    fn from(refusal: SpawnRefusal) -> Self {
        match refusal {
            SpawnRefusal::AtCapacity { limit } => Self::AtCapacity { limit },
            SpawnRefusal::DepthExceeded { depth, max } => Self::DepthExceeded { depth, max },
        }
    }
}

impl From<SpawnToolError> for McpError {
    fn from(error: SpawnToolError) -> Self {
        match error {
            SpawnToolError::InvalidArgument(message) => McpError::invalid_params(
                message,
                Some(json!({ "code": "invalid-argument" })),
            ),
            // P#23: a distinct at-capacity error naming the limit; nothing is
            // queued.
            SpawnToolError::AtCapacity { limit } => McpError::invalid_params(
                format!(
                    "At capacity: {limit} spawned sessions are already running (limit {limit}); retry after one completes"
                ),
                Some(json!({ "code": "at-capacity", "limit": limit })),
            ),
            // P#24: a distinct depth-exceeded error.
            SpawnToolError::DepthExceeded { depth, max } => McpError::invalid_params(
                format!("Spawn depth {depth} exceeds the maximum chain depth {max}"),
                Some(json!({ "code": "depth-exceeded", "depth": depth, "max": max })),
            ),
            SpawnToolError::Internal(message) => {
                McpError::internal_error(message, Some(json!({ "code": "internal" })))
            }
        }
    }
}

impl SessionsMcpBridge {
    /// 26d: validate and spawn a `create_chat` session on the main thread
    /// (PRODUCT P#13–16, 22–24, 26–28). Validation happens atomically against
    /// the registry via [`SessionRegistry::try_reserve_spawn`] — the cap slot
    /// is reserved under the registry lock before the pane exists, so racing
    /// calls can never exceed the cap. Returns the new session id immediately
    /// (P#13); nothing here touches permission plumbing, so a spawned
    /// session's prompts behave exactly like a user-opened one's (P#26).
    fn create_chat(
        &mut self,
        params: CreateChatParams,
        parent: SpawnParent,
        ctx: &mut ModelContext<Self>,
    ) -> Result<serde_json::Value, SpawnToolError> {
        let cwd = PathBuf::from(&params.cwd);
        // P#15: cwd must be an existing directory; otherwise nothing is
        // created.
        if !cwd.is_dir() {
            return Err(SpawnToolError::InvalidArgument(format!(
                "cwd is not an existing directory: {}",
                params.cwd
            )));
        }
        let settings = AgentSettings::as_ref(ctx);
        let chat_config = settings.chat_launch_config();
        let cap = *settings.sessions_spawn_cap.value();
        // P#13: provider defaults to the app's configured default.
        let provider = match params.provider.as_deref() {
            None => chat_config
                .provider
                .agent_provider()
                .unwrap_or(claude_code::driver::AgentProvider::Claude),
            Some("claude") => claude_code::driver::AgentProvider::Claude,
            Some("codex") => claude_code::driver::AgentProvider::Codex,
            Some(other) => {
                return Err(SpawnToolError::InvalidArgument(format!(
                    "provider must be claude or codex, got: {other}"
                )));
            }
        };
        // P#15: a named project must already exist — create_chat never
        // implicitly creates one.
        let project_root = params
            .project
            .as_deref()
            .map(|project| {
                crate::projects::ProjectManagementModel::as_ref(ctx)
                    .all_projects()
                    .find(|candidate| project_display_name(candidate) == project)
                    .map(|candidate| PathBuf::from(&candidate.path))
                    .ok_or_else(|| {
                        SpawnToolError::InvalidArgument(format!(
                            "No project named {project} exists"
                        ))
                    })
            })
            .transpose()?;

        // P#23–24, 28: reserve the cap slot and mint the provenance under one
        // registry lock, keyed by the id the fresh pane will be pinned to.
        let session_id = uuid::Uuid::new_v4().to_string();
        let registry = SessionRegistry::global();
        let origin = registry.try_reserve_spawn(&session_id, &parent, cap)?;

        let workspace = ctx
            .windows()
            .active_window()
            .or_else(|| ctx.windows().frontmost_window_id())
            .and_then(|window_id| WorkspaceRegistry::as_ref(ctx).get(window_id, ctx))
            .or_else(|| {
                WorkspaceRegistry::as_ref(ctx)
                    .all_workspaces(ctx)
                    .into_iter()
                    .next()
                    .map(|(_, workspace)| workspace)
            });
        let Some(workspace) = workspace else {
            // Release the reserved slot — nothing was spawned.
            registry.remove(&session_id);
            return Err(SpawnToolError::Internal(
                "No workspace is available to open an agent pane".to_owned(),
            ));
        };

        let launch = claude_code::launch::LaunchOptions {
            provider,
            // P#13: the prompt is submitted as the first user message via the
            // same submit path the composer uses — never PTY bytes.
            prompt: Some(params.prompt),
            permission_mode: Some(chat_config.permission_mode),
            model: params.model.clone().or(chat_config.model),
            effort: chat_config.effort,
            resume_session_id: None,
            pinned_session_id: Some(session_id.clone()),
        };
        workspace.update(ctx, |workspace, ctx| {
            workspace.open_spawned_agent_chat(
                launch,
                cwd.clone(),
                project_root,
                origin.clone(),
                ctx,
            );
        });

        Ok(json!({
            "session_id": session_id,
            "provider": provider.as_persistence_str(),
            "cwd": cwd.to_string_lossy(),
            "spawned_by": origin.parent_label,
            "depth": origin.depth,
        }))
    }

    /// 26d: create a sidebar project on the main thread (PRODUCT P#18) — the
    /// same `create_project(NewProjectSource::ExistingFolder)` path the UI
    /// uses. Duplicate (an existing project for the same folder) errors.
    fn create_project(
        &mut self,
        params: CreateProjectParams,
        ctx: &mut ModelContext<Self>,
    ) -> Result<serde_json::Value, SpawnToolError> {
        let name = params.name.trim().to_owned();
        if name.is_empty() {
            return Err(SpawnToolError::InvalidArgument(
                "name must not be empty".to_owned(),
            ));
        }
        let root = PathBuf::from(&params.cwd);
        if !root.is_dir() {
            return Err(SpawnToolError::InvalidArgument(format!(
                "cwd is not an existing directory: {}",
                params.cwd
            )));
        }
        let identity = crate::projects::project_identity(root.clone());
        let duplicate = crate::projects::ProjectManagementModel::as_ref(ctx)
            .all_projects()
            .any(|project| PathBuf::from(&project.path) == identity);
        if duplicate {
            return Err(SpawnToolError::InvalidArgument(format!(
                "A project already exists for {}",
                identity.to_string_lossy()
            )));
        }

        let workspace = ctx
            .windows()
            .active_window()
            .or_else(|| ctx.windows().frontmost_window_id())
            .and_then(|window_id| WorkspaceRegistry::as_ref(ctx).get(window_id, ctx))
            .or_else(|| {
                WorkspaceRegistry::as_ref(ctx)
                    .all_workspaces(ctx)
                    .into_iter()
                    .next()
                    .map(|(_, workspace)| workspace)
            })
            .ok_or_else(|| {
                SpawnToolError::Internal(
                    "No workspace is available to create a project".to_owned(),
                )
            })?;
        workspace.update(ctx, |workspace, ctx| {
            workspace.create_project_for_sessions_mcp(name.clone(), identity.clone(), ctx);
        });

        Ok(json!({
            "name": name,
            // Projects carry no persisted color (color is per-tab); a color
            // argument is accepted and ignored.
            "color": serde_json::Value::Null,
            "cwd": identity.to_string_lossy(),
            "session_count": claude_code::sessions::list_sessions(&identity).len(),
        }))
    }
}

/// A project's display name, matching `list_projects`' projection: the custom
/// name, or the folder name, or the raw path.
fn project_display_name(project: &crate::projects::Project) -> String {
    project
        .name
        .clone()
        .filter(|name| !name.trim().is_empty())
        .or_else(|| {
            Path::new(&project.path)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| project.path.clone())
}

impl Entity for SessionsMcpBridge {
    type Event = ();
}

impl SingletonEntity for SessionsMcpBridge {}

impl SessionsMcpRuntime {
    fn start(spawner: ModelSpawner<SessionsMcpBridge>) -> Result<Self, String> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .map_err(|err| err.to_string())?;
        let cancel = CancellationToken::new();
        let url = Self::start_server(
            runtime.handle(),
            spawner.clone(),
            cancel.clone(),
            None,
            ListenerKind::InApp,
        )?;
        Ok(Self {
            url,
            spawner,
            session_servers: StdMutex::new(HashMap::new()),
            external: StdMutex::new(None),
            runtime,
            cancel,
        })
    }

    /// 26d: the port the external listener currently serves, if running.
    fn external_port(&self) -> Option<u16> {
        self.external
            .lock()
            .ok()
            .and_then(|external| external.as_ref().map(|listener| listener.port))
    }

    /// 26d: bind the external token-gated listener on the fixed port
    /// (127.0.0.1 only). Errors (typically a port conflict) bubble to the
    /// settings page (PRODUCT P#30).
    fn start_external(&self, port: u16) -> Result<(), String> {
        let cancel = self.cancel.child_token();
        Self::start_server(
            self.runtime.handle(),
            self.spawner.clone(),
            cancel.clone(),
            None,
            ListenerKind::External { port },
        )?;
        let mut external = self
            .external
            .lock()
            .map_err(|_| "sessions MCP external-listener state is poisoned".to_owned())?;
        *external = Some(ExternalListener { cancel, port });
        Ok(())
    }

    /// 26d: stop the external listener. Cancelling its child token shuts the
    /// HTTP server down and severs its live watch/wait connections (PRODUCT
    /// P#12, 21).
    fn stop_external(&self) {
        if let Ok(mut external) = self.external.lock() {
            if let Some(listener) = external.take() {
                listener.cancel.cancel();
            }
        }
    }

    /// Returns the SSE URL of the given session's dedicated server, starting
    /// it on first use.
    fn session_server_url(&self, session_id: &str) -> Result<String, String> {
        let mut servers = self
            .session_servers
            .lock()
            .map_err(|_| "sessions MCP session-server registry is poisoned".to_owned())?;
        if let Some(url) = servers.get(session_id) {
            return Ok(url.clone());
        }
        let url = Self::start_server(
            self.runtime.handle(),
            self.spawner.clone(),
            self.cancel.child_token(),
            Some(session_id.to_owned()),
            ListenerKind::InApp,
        )?;
        servers.insert(session_id.to_owned(), url.clone());
        Ok(url)
    }

    /// Binds a localhost SSE server on an ephemeral port and returns its URL.
    /// `scope_session_id` stamps every service instance created for this
    /// endpoint with the owning Claude session.
    fn start_server(
        handle: &tokio::runtime::Handle,
        spawner: ModelSpawner<SessionsMcpBridge>,
        cancel: CancellationToken,
        scope_session_id: Option<String>,
        kind: ListenerKind,
    ) -> Result<String, String> {
        let (addr_tx, addr_rx) = mpsc::channel();
        let server_cancel = cancel.clone();
        let (bind_port, is_external) = match kind {
            ListenerKind::InApp => (0, false),
            ListenerKind::External { port } => (port, true),
        };

        handle.spawn(async move {
            use rmcp::transport::sse_server::{SseServer, SseServerConfig};

            let config = SseServerConfig {
                // 127.0.0.1 only — the external surface is localhost-bound by
                // construction (PRODUCT: no cloud/remote transport).
                bind: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), bind_port),
                sse_path: SSE_PATH.to_owned(),
                post_path: POST_PATH.to_owned(),
                ct: server_cancel.clone(),
                sse_keep_alive: None,
            };
            let (sse_server, router) = SseServer::new(config);
            // 26d: the external listener rejects every request without the
            // current bearer token (PRODUCT P#21, 27).
            let router = if is_external {
                router.layer(axum::middleware::from_fn(external::require_bearer_token))
            } else {
                router
            };
            let listener = match tokio::net::TcpListener::bind(sse_server.config.bind).await {
                Ok(listener) => listener,
                Err(err) => {
                    let _ = addr_tx.send(Err(err.to_string()));
                    return;
                }
            };
            let local_addr = match listener.local_addr() {
                Ok(addr) => addr,
                Err(err) => {
                    let _ = addr_tx.send(Err(err.to_string()));
                    return;
                }
            };
            let _ = addr_tx.send(Ok(local_addr));

            let shutdown = sse_server.config.ct.child_token();
            let server = axum::serve(listener, router).with_graceful_shutdown(async move {
                shutdown.cancelled().await;
            });
            tokio::spawn(async move {
                if let Err(err) = server.await {
                    log::warn!("twarp sessions MCP HTTP server stopped with error: {err}");
                }
            });

            let _service_cancel = sse_server.with_service(move || {
                SessionsMcpServer::new(spawner.clone(), scope_session_id.clone(), is_external)
            });
            server_cancel.cancelled().await;
        });

        let addr = addr_rx
            .recv_timeout(Duration::from_secs(2))
            .map_err(|err| err.to_string())??;
        Ok(format!("http://{addr}{SSE_PATH}"))
    }
}

/// Which listener a service instance is created for: the tokenless in-app
/// ephemeral-port servers, or the token-gated external fixed-port listener.
#[derive(Clone, Copy)]
enum ListenerKind {
    InApp,
    External { port: u16 },
}

#[derive(Clone)]
struct SessionsMcpServer {
    spawner: ModelSpawner<SessionsMcpBridge>,
    tool_router: ToolRouter<Self>,
    /// The Claude session this endpoint belongs to (None on the shared and
    /// external endpoints). 26d: `create_chat`'s in-pane caller identity — the
    /// scoped session is the spawned session's parent (PRODUCT P#16).
    scope_session_id: Option<String>,
    /// 26d: whether this service serves the external token-gated listener
    /// (its spawns are recorded with an external origin at depth 1).
    external: bool,
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for SessionsMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            server_info: Implementation {
                name: SERVER_NAME.to_owned(),
                title: Some("twarp Sessions".to_owned()),
                version: env!("CARGO_PKG_VERSION").to_owned(),
                icons: None,
                website_url: None,
            },
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .enable_logging()
                .build(),
            instructions: Some(
                "Control and monitor twarp's agent sessions and projects: create new chat sessions (create_chat) and sidebar projects (create_project), list live and stored chat sessions with their status, read any session's transcript, watch a session's live events, and wait for a session to complete. Statuses match the twarp tab UI exactly."
                    .to_owned(),
            ),
            ..Default::default()
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ListSessionsParams {
    /// Also include stored past sessions from both providers' on-disk stores
    /// (past sessions always report status "idle").
    include_past: Option<bool>,
    /// Filter to one status: running | needs_input | done_ok | done_error | idle.
    status: Option<String>,
    /// Filter to one provider: claude | codex.
    provider: Option<String>,
    /// Filter to sessions whose working directory equals this path.
    cwd: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct GetTranscriptParams {
    session_id: String,
    /// Return only items with index greater than this (the latest index
    /// yields an empty list). Omit for the full transcript.
    since_index: Option<u64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct WatchSessionParams {
    /// The live session to watch.
    session_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct WaitForCompletionParams {
    /// A live session id, or "any" to wait on every live session at once.
    session_id: String,
    /// How long to wait before returning a timed_out result. Defaults to 300
    /// seconds; capped at 3600.
    timeout_seconds: Option<f64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CreateChatParams {
    /// The first user message of the new session.
    prompt: String,
    /// The new session's working directory. Must be an existing directory.
    cwd: String,
    /// claude | codex. Defaults to the app's configured chat provider.
    provider: Option<String>,
    /// Model id override. Defaults to the provider's configured default.
    model: Option<String>,
    /// Name of an existing sidebar project to file the new session under.
    /// Errors when no such project exists (never implicitly created).
    project: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CreateProjectParams {
    /// The project's display name.
    name: String,
    /// The project's folder. Must be an existing directory; a project already
    /// registered for this folder is a duplicate and errors.
    cwd: String,
    /// Accepted for compatibility but ignored: twarp projects have no
    /// persisted color (color is a per-tab property).
    #[allow(dead_code)]
    color: Option<String>,
}

/// `wait_for_completion` timeout bounds (PRODUCT P#29: nothing may hang
/// indefinitely, so the cap is hard).
const DEFAULT_WAIT_TIMEOUT: Duration = Duration::from_secs(300);
const MAX_WAIT_TIMEOUT: Duration = Duration::from_secs(3600);

#[derive(Debug, Serialize)]
struct SessionRow {
    session_id: String,
    provider: String,
    cwd: Option<String>,
    title: String,
    status: &'static str,
    is_live: bool,
    last_activity: String,
    project: Option<String>,
    /// 26d: provenance for sessions created via `create_chat` (the creator's
    /// label) and their spawn-chain depth. Null for user-opened sessions.
    spawned_by: Option<String>,
    spawn_depth: Option<u8>,
}

#[tool_router(router = tool_router)]
impl SessionsMcpServer {
    fn new(
        spawner: ModelSpawner<SessionsMcpBridge>,
        scope_session_id: Option<String>,
        external: bool,
    ) -> Self {
        Self {
            spawner,
            tool_router: Self::tool_router(),
            scope_session_id,
            external,
        }
    }

    #[tool(
        name = "list_sessions",
        description = "List twarp agent sessions: every live agent pane, plus stored past sessions when include_past is true. Each entry carries session_id, provider (claude|codex), cwd, title, status (running|needs_input|done_ok|done_error|idle), is_live, and last activity. Optional status/provider/cwd filters."
    )]
    async fn list_sessions(
        &self,
        Parameters(params): Parameters<ListSessionsParams>,
    ) -> Result<CallToolResult, McpError> {
        let status_filter = params
            .status
            .as_deref()
            .map(|status| {
                SessionStatus::parse(status).ok_or_else(|| {
                    McpError::invalid_params(
                        "status must be one of running, needs_input, done_ok, done_error, idle",
                        Some(json!({ "code": "invalid-argument", "status": status })),
                    )
                })
            })
            .transpose()?;
        let provider_filter = params
            .provider
            .as_deref()
            .map(|provider| match provider {
                "claude" | "codex" => Ok(provider.to_owned()),
                other => Err(McpError::invalid_params(
                    "provider must be claude or codex",
                    Some(json!({ "code": "invalid-argument", "provider": other })),
                )),
            })
            .transpose()?;
        let cwd_filter = params.cwd.map(PathBuf::from);

        let live = SessionRegistry::global().snapshot();
        let live_ids: std::collections::HashSet<String> = live
            .iter()
            .map(|session| session.session_id.clone())
            .collect();
        let mut rows: Vec<SessionRow> = live
            .into_iter()
            .map(|session| SessionRow {
                session_id: session.session_id,
                provider: session.provider.to_owned(),
                cwd: session
                    .cwd
                    .as_deref()
                    .map(|cwd| cwd.to_string_lossy().into_owned()),
                title: session.title,
                status: session.status.as_str(),
                is_live: true,
                last_activity: rfc3339(session.last_activity),
                project: None,
                spawned_by: session
                    .origin
                    .as_ref()
                    .map(|origin| origin.parent_label.clone()),
                spawn_depth: session.origin.as_ref().map(|origin| origin.depth),
            })
            .collect();

        if params.include_past.unwrap_or(false) {
            let past = tokio::task::spawn_blocking(store::all_past_sessions)
                .await
                .map_err(|err| McpError::internal_error(err.to_string(), None))?;
            rows.extend(
                past.into_iter()
                    // A live pane's session shadows its stored copy.
                    .filter(|past| !live_ids.contains(&past.session.id))
                    .map(|past| SessionRow {
                        session_id: past.session.id,
                        provider: past.session.provider.as_persistence_str().to_owned(),
                        cwd: past
                            .cwd
                            .as_deref()
                            .map(|cwd| cwd.to_string_lossy().into_owned()),
                        title: past.session.title,
                        // PRODUCT P#3: past (non-live) sessions always idle.
                        status: SessionStatus::Idle.as_str(),
                        is_live: false,
                        last_activity: rfc3339(past.session.timestamp),
                        project: None,
                        spawned_by: None,
                        spawn_depth: None,
                    }),
            );
        }

        rows.retain(|row| {
            status_filter.is_none_or(|status| row.status == status.as_str())
                && provider_filter
                    .as_deref()
                    .is_none_or(|provider| row.provider == provider)
                && cwd_filter.as_deref().is_none_or(|cwd| {
                    row.cwd
                        .as_deref()
                        .is_some_and(|row_cwd| Path::new(row_cwd) == cwd)
                })
        });

        json_result(json!({ "sessions": rows }))
    }

    #[tool(
        name = "get_transcript",
        description = "Return a session's transcript as ordered items with stable monotonically increasing indices (index, role, text). Works for live and stored sessions of both providers. Pass since_index to get only items after that index."
    )]
    async fn get_transcript(
        &self,
        Parameters(params): Parameters<GetTranscriptParams>,
    ) -> Result<CallToolResult, McpError> {
        if let Some(items) =
            SessionRegistry::global().transcript_since(&params.session_id, params.since_index)
        {
            return json_result(json!({
                "session_id": params.session_id,
                "is_live": true,
                "items": items,
            }));
        }

        let session_id = params.session_id.clone();
        let stored = tokio::task::spawn_blocking(move || {
            store::find_stored_session(&session_id)
                .map(|past| store::stored_transcript(&past.session))
        })
        .await
        .map_err(|err| McpError::internal_error(err.to_string(), None))?;
        let Some(items) = stored else {
            return Err(McpError::invalid_params(
                "No live or stored session exists with that id",
                Some(json!({ "code": "not-found", "session_id": params.session_id })),
            ));
        };
        json_result(json!({
            "session_id": params.session_id,
            "is_live": false,
            "items": items_since(&items, params.since_index),
        }))
    }

    #[tool(
        name = "watch_session",
        description = "Subscribe this connection to a live session: subsequent transcript items and status changes arrive as notifications/message MCP notifications (logger \"twarp-sessions\", data.type \"session_event\") until the session closes. A terminal {event: \"closed\"} notification is sent when the pane closes."
    )]
    async fn watch_session(
        &self,
        peer: Peer<RoleServer>,
        Parameters(params): Parameters<WatchSessionParams>,
    ) -> Result<CallToolResult, McpError> {
        let Some(subscription) = SessionRegistry::global().subscribe(&params.session_id) else {
            return Err(McpError::invalid_params(
                "No live session exists with that id",
                Some(json!({ "code": "not-found", "session_id": params.session_id })),
            ));
        };
        let status = subscription.status;

        // The forwarder produces notification payloads into a channel; the
        // drain task pushes them onto this SSE connection. When the client
        // disconnects the notify fails, the receiver drops, and the
        // forwarder's next send unblocks it into exit — nothing hangs (P#29).
        let (tx, mut rx) = tokio::sync::mpsc::channel::<WatchNotification>(64);
        tokio::spawn(events::forward_watch_events(
            SessionRegistry::global(),
            params.session_id.clone(),
            subscription,
            tx,
        ));
        tokio::spawn(async move {
            while let Some(WatchNotification(data)) = rx.recv().await {
                if peer
                    .notify_logging_message(LoggingMessageNotificationParam {
                        level: LoggingLevel::Info,
                        logger: Some(SERVER_NAME.to_owned()),
                        data,
                    })
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });

        json_result(json!({
            "watching": params.session_id,
            "status": status.as_str(),
        }))
    }

    #[tool(
        name = "wait_for_completion",
        description = "Block until a live session (or any live session, with session_id \"any\") transitions to done_ok, done_error, or needs_input, then return its final status and last assistant message text. A closed pane resolves as done_error with reason \"closed\". On timeout returns result \"timed_out\" (not an error)."
    )]
    async fn wait_for_completion(
        &self,
        Parameters(params): Parameters<WaitForCompletionParams>,
    ) -> Result<CallToolResult, McpError> {
        let registry = SessionRegistry::global();
        let targets = if params.session_id == "any" {
            let targets = registry.subscribe_all();
            if targets.is_empty() {
                return Err(McpError::invalid_params(
                    "No live sessions to wait on",
                    Some(json!({ "code": "not-found", "session_id": "any" })),
                ));
            }
            targets
        } else {
            let Some(subscription) = registry.subscribe(&params.session_id) else {
                return Err(McpError::invalid_params(
                    "No live session exists with that id",
                    Some(json!({ "code": "not-found", "session_id": params.session_id })),
                ));
            };
            vec![(params.session_id.clone(), subscription)]
        };

        let timeout = match params.timeout_seconds {
            None => DEFAULT_WAIT_TIMEOUT,
            Some(seconds) if seconds.is_finite() && seconds >= 0.0 => {
                Duration::from_secs_f64(seconds).min(MAX_WAIT_TIMEOUT)
            }
            Some(seconds) => {
                return Err(McpError::invalid_params(
                    "timeout_seconds must be a non-negative number",
                    Some(json!({ "code": "invalid-argument", "timeout_seconds": seconds })),
                ));
            }
        };
        match events::wait_for_completion(registry, targets, timeout).await {
            WaitOutcome::Completed {
                session_id,
                status,
                reason,
                last_assistant_text,
            } => json_result(json!({
                "result": "completed",
                "session_id": session_id,
                "status": status.as_str(),
                "reason": reason,
                "last_assistant_text": last_assistant_text,
            })),
            // PRODUCT P#10: a distinct timed_out result, not an error.
            WaitOutcome::TimedOut => json_result(json!({ "result": "timed_out" })),
        }
    }

    #[tool(
        name = "list_projects",
        description = "List twarp's sidebar projects: name, color, cwd, and the count of stored sessions in each project's directory."
    )]
    async fn list_projects(&self) -> Result<CallToolResult, McpError> {
        let projects = self
            .spawner
            .spawn(|bridge, ctx| bridge.list_projects(ctx))
            .await
            .map_err(|_| McpError::internal_error("Sessions MCP bridge is unavailable", None))?;
        json_result(json!({ "projects": projects }))
    }

    #[tool(
        name = "create_chat",
        description = "Open a new agent chat pane in a new tab, submit the prompt as its first user message, and return the new session_id immediately (combine with wait_for_completion for the reply). Validated against the spawn cap (distinct at-capacity error) and spawn-chain depth (distinct depth-exceeded error); cwd must exist and a given project must already exist."
    )]
    async fn create_chat(
        &self,
        Parameters(params): Parameters<CreateChatParams>,
    ) -> Result<CallToolResult, McpError> {
        // P#16: the tool's scoped session is the parent; external consumers
        // record an external origin ("external" when unnamed) at depth 1.
        let parent = match (&self.scope_session_id, self.external) {
            (Some(session_id), _) => {
                let title = SessionRegistry::global()
                    .snapshot()
                    .into_iter()
                    .find(|session| &session.session_id == session_id)
                    .map(|session| session.title)
                    .filter(|title| !title.trim().is_empty())
                    .unwrap_or_else(|| session_id.clone());
                SpawnParent::InPane {
                    session_id: session_id.clone(),
                    title,
                }
            }
            // No consumer-naming mechanism exists yet, so external callers
            // are labeled "external"; the unscoped in-app fallback endpoint
            // is distinguished as "in-app".
            (None, true) => SpawnParent::External {
                label: "external".to_owned(),
            },
            (None, false) => SpawnParent::External {
                label: "in-app".to_owned(),
            },
        };
        let result = self
            .spawner
            .spawn(move |bridge, ctx| bridge.create_chat(params, parent, ctx))
            .await
            .map_err(|_| McpError::internal_error("Sessions MCP bridge is unavailable", None))?;
        json_result(result.map_err(McpError::from)?)
    }

    #[tool(
        name = "create_project",
        description = "Create a sidebar project (name + existing folder), exactly as through the UI, and return it. A project already registered for the folder is a duplicate and errors. Note: twarp projects have no persisted color; a color argument is accepted but ignored and the returned color is null."
    )]
    async fn create_project(
        &self,
        Parameters(params): Parameters<CreateProjectParams>,
    ) -> Result<CallToolResult, McpError> {
        let result = self
            .spawner
            .spawn(move |bridge, ctx| bridge.create_project(params, ctx))
            .await
            .map_err(|_| McpError::internal_error("Sessions MCP bridge is unavailable", None))?;
        json_result(result.map_err(McpError::from)?)
    }
}

fn rfc3339(time: SystemTime) -> String {
    chrono::DateTime::<chrono::Utc>::from(time).to_rfc3339()
}

fn json_result<T: Serialize>(value: T) -> Result<CallToolResult, McpError> {
    let value = serde_json::to_value(value).map_err(|err| {
        McpError::internal_error(
            "Failed to serialize sessions tool result",
            Some(json!({ "error": err.to_string() })),
        )
    })?;
    let text = serde_json::to_string_pretty(&value).map_err(|err| {
        McpError::internal_error(
            "Failed to render sessions tool result",
            Some(json!({ "error": err.to_string() })),
        )
    })?;
    let mut result = CallToolResult::success(vec![Content::text(text)]);
    result.structured_content = Some(value);
    Ok(result)
}
