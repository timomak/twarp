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
    registry::{items_since, SessionRegistry},
    status::SessionStatus,
    store,
};

const SERVER_NAME: &str = "twarp-sessions";
const SSE_PATH: &str = "/sse";
const POST_PATH: &str = "/message";

pub(crate) struct SessionsMcpBridge {
    server: Option<SessionsMcpRuntime>,
}

struct SessionsMcpRuntime {
    url: String,
    spawner: ModelSpawner<SessionsMcpBridge>,
    /// One extra SSE server per Claude session (keyed by session id), so
    /// every tool call carries the calling session's identity — the same
    /// scoping the other built-in servers use.
    session_servers: StdMutex<HashMap<String, String>>,
    runtime: tokio::runtime::Runtime,
    cancel: CancellationToken,
}

impl SessionsMcpBridge {
    pub(crate) fn new(ctx: &mut ModelContext<Self>) -> Self {
        let spawner = ctx.spawner();
        let server = SessionsMcpRuntime::start(spawner)
            .inspect_err(|err| log::warn!("Failed to start twarp sessions MCP server: {err}"))
            .ok();

        Self { server }
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
        let url = Self::start_server(runtime.handle(), spawner.clone(), cancel.clone(), None)?;
        Ok(Self {
            url,
            spawner,
            session_servers: StdMutex::new(HashMap::new()),
            runtime,
            cancel,
        })
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
    ) -> Result<String, String> {
        let (addr_tx, addr_rx) = mpsc::channel();
        let server_cancel = cancel.clone();

        handle.spawn(async move {
            use rmcp::transport::sse_server::{SseServer, SseServerConfig};

            let config = SseServerConfig {
                bind: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
                sse_path: SSE_PATH.to_owned(),
                post_path: POST_PATH.to_owned(),
                ct: server_cancel.clone(),
                sse_keep_alive: None,
            };
            let (sse_server, router) = SseServer::new(config);
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
                SessionsMcpServer::new(spawner.clone(), scope_session_id.clone())
            });
            server_cancel.cancelled().await;
        });

        let addr = addr_rx
            .recv_timeout(Duration::from_secs(2))
            .map_err(|err| err.to_string())??;
        Ok(format!("http://{addr}{SSE_PATH}"))
    }
}

#[derive(Clone)]
struct SessionsMcpServer {
    spawner: ModelSpawner<SessionsMcpBridge>,
    tool_router: ToolRouter<Self>,
    /// The Claude session this endpoint belongs to (None on the shared
    /// endpoint). Read tools don't scope on it yet; 26c–26d provenance and
    /// spawn depth will.
    #[allow(dead_code)]
    scope_session_id: Option<String>,
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
                "Read and monitor twarp's agent sessions and projects: list live and stored chat sessions with their status, read any session's transcript, list sidebar projects, watch a session's live events, and wait for a session to complete. Statuses match the twarp tab UI exactly."
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
}

#[tool_router(router = tool_router)]
impl SessionsMcpServer {
    fn new(spawner: ModelSpawner<SessionsMcpBridge>, scope_session_id: Option<String>) -> Self {
        Self {
            spawner,
            tool_router: Self::tool_router(),
            scope_session_id,
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
