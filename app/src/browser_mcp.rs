use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::mpsc,
    time::Duration,
};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use browser::WaitSpec;
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Content, Implementation, ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio_util::sync::CancellationToken;
use twarpui::{AppContext, Entity, ModelContext, ModelSpawner, SingletonEntity};

use crate::{
    browser_view::{normalize_browser_url, BrowserAutomationTarget, BrowserView},
    workspace::WorkspaceRegistry,
};

const SERVER_NAME: &str = "twarp-browser";
const SSE_PATH: &str = "/sse";
const POST_PATH: &str = "/message";
/// How long browser_navigate waits for a freshly opened pane to register.
const NEW_PANE_RESOLVE_TIMEOUT: Duration = Duration::from_secs(5);
const NEW_PANE_RESOLVE_POLL_INTERVAL: Duration = Duration::from_millis(100);
/// How long browser_navigate waits for the new pane's initial page to settle.
const NEW_PANE_SETTLE_TIMEOUT: Duration = Duration::from_secs(15);

pub(crate) struct BrowserMcpBridge {
    server: Option<BrowserMcpRuntime>,
}

struct BrowserMcpRuntime {
    url: String,
    _runtime: tokio::runtime::Runtime,
    _cancel: CancellationToken,
}

impl BrowserMcpBridge {
    pub(crate) fn new(ctx: &mut ModelContext<Self>) -> Self {
        let spawner = ctx.spawner();
        let server = BrowserMcpRuntime::start(spawner)
            .inspect_err(|err| log::warn!("Failed to start twarp browser MCP server: {err}"))
            .ok();

        Self { server }
    }

    pub(crate) fn mcp_config_json(&self) -> Option<String> {
        let url = &self.server.as_ref()?.url;
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

    fn resolve_target(&self, ctx: &AppContext) -> Option<BrowserAutomationTarget> {
        ctx.window_ids()
            .filter_map(|window_id| ctx.views_of_type::<BrowserView>(window_id))
            .flatten()
            .filter_map(|view| view.as_ref(ctx).automation_target())
            .max_by_key(|target| target.last_focus_seq)
    }

    fn open_browser_pane(
        &mut self,
        url: String,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), String> {
        let active_window = ctx
            .windows()
            .active_window()
            .or_else(|| ctx.windows().frontmost_window_id());

        let workspace = active_window
            .and_then(|window_id| {
                WorkspaceRegistry::as_ref(ctx)
                    .get(window_id, ctx)
                    .map(|workspace| (window_id, workspace))
            })
            .or_else(|| {
                WorkspaceRegistry::as_ref(ctx)
                    .all_workspaces(ctx)
                    .into_iter()
                    .next()
            })
            .ok_or_else(|| "No workspace is available to open a Browser pane".to_owned())?;

        let (window_id, workspace) = workspace;
        ctx.windows().show_window_and_focus_app(window_id);
        workspace.update(ctx, |workspace, ctx| {
            workspace.open_browser_pane(Some(url), ctx);
        });
        Ok(())
    }
}

impl Entity for BrowserMcpBridge {
    type Event = ();
}

impl SingletonEntity for BrowserMcpBridge {}

impl BrowserMcpRuntime {
    fn start(spawner: ModelSpawner<BrowserMcpBridge>) -> Result<Self, String> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .map_err(|err| err.to_string())?;
        let cancel = CancellationToken::new();
        let (addr_tx, addr_rx) = mpsc::channel();
        let server_cancel = cancel.clone();

        runtime.spawn(async move {
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
                    log::warn!("twarp browser MCP HTTP server stopped with error: {err}");
                }
            });

            let _service_cancel =
                sse_server.with_service(move || BrowserMcpServer::new(spawner.clone()));
            server_cancel.cancelled().await;
        });

        let addr = addr_rx
            .recv_timeout(Duration::from_secs(2))
            .map_err(|err| err.to_string())??;
        Ok(Self {
            url: format!("http://{addr}{SSE_PATH}"),
            _runtime: runtime,
            _cancel: cancel,
        })
    }
}

#[derive(Clone)]
struct BrowserMcpServer {
    spawner: ModelSpawner<BrowserMcpBridge>,
    tool_router: ToolRouter<Self>,
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for BrowserMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            server_info: Implementation {
                name: SERVER_NAME.to_owned(),
                title: Some("twarp Browser".to_owned()),
                version: env!("CARGO_PKG_VERSION").to_owned(),
                icons: None,
                website_url: None,
            },
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            instructions: Some(
                "Drive the live twarp Browser pane that the user sees. Tools target the most recently focused Browser pane; browser_navigate opens one if needed."
                    .to_owned(),
            ),
            ..Default::default()
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct NavigateParams {
    url: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RefParams {
    #[serde(rename = "ref")]
    reference: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct TypeParams {
    #[serde(rename = "ref")]
    reference: String,
    text: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct EvalParams {
    js: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct WaitParams {
    selector: Option<String>,
    navigation: Option<bool>,
    timeout_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
struct ToolEnvelope<T> {
    target: String,
    result: T,
}

#[tool_router(router = tool_router)]
impl BrowserMcpServer {
    fn new(spawner: ModelSpawner<BrowserMcpBridge>) -> Self {
        Self {
            spawner,
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        name = "browser_navigate",
        description = "Navigate the live twarp Browser pane to a URL. Opens a Browser pane if none exists."
    )]
    async fn browser_navigate(
        &self,
        Parameters(params): Parameters<NavigateParams>,
    ) -> Result<CallToolResult, McpError> {
        let url = normalize_browser_url(&params.url).ok_or_else(|| {
            McpError::invalid_params(
                "browser_navigate requires a URL with a scheme, host, localhost, or dotted host",
                Some(json!({ "url": params.url })),
            )
        })?;

        match self.resolve_target().await? {
            Some(target) => {
                target.engine.navigate(&url).await.map_err(tool_error)?;
                json_result(ToolEnvelope {
                    target: target.label(),
                    result: json!({ "url": url }),
                })
            }
            None => {
                self.open_browser_pane(url.clone()).await?;
                // The pane (and its initial navigation) is created
                // asynchronously on the main thread; wait for it to register
                // and for the page to settle so follow-up tools (snapshot,
                // type, eval) don't race the load.
                let deadline = std::time::Instant::now() + NEW_PANE_RESOLVE_TIMEOUT;
                let target = loop {
                    if let Some(target) = self.resolve_target().await? {
                        break Some(target);
                    }
                    if std::time::Instant::now() >= deadline {
                        break None;
                    }
                    tokio::time::sleep(NEW_PANE_RESOLVE_POLL_INTERVAL).await;
                };
                if let Some(target) = target {
                    let _ = target
                        .engine
                        .wait(WaitSpec::NavigationSettled {
                            timeout: NEW_PANE_SETTLE_TIMEOUT,
                        })
                        .await;
                }
                json_result(json!({
                    "target": "new Browser pane",
                    "result": { "url": url }
                }))
            }
        }
    }

    #[tool(
        name = "browser_snapshot",
        description = "Return a DOM/accessibility snapshot with stable element refs for the live twarp Browser pane."
    )]
    async fn browser_snapshot(&self) -> Result<CallToolResult, McpError> {
        let target = self.required_target().await?;
        let snapshot = target.engine.snapshot().await.map_err(tool_error)?;
        json_result(ToolEnvelope {
            target: target.label(),
            result: snapshot,
        })
    }

    #[tool(
        name = "browser_click",
        description = "Click an element from the latest browser_snapshot ref."
    )]
    async fn browser_click(
        &self,
        Parameters(params): Parameters<RefParams>,
    ) -> Result<CallToolResult, McpError> {
        let target = self.required_target().await?;
        let result = target
            .engine
            .click(&params.reference)
            .await
            .map_err(tool_error)?;
        json_result(ToolEnvelope {
            target: target.label(),
            result,
        })
    }

    #[tool(
        name = "browser_type",
        description = "Focus an element from a browser_snapshot ref and type text."
    )]
    async fn browser_type(
        &self,
        Parameters(params): Parameters<TypeParams>,
    ) -> Result<CallToolResult, McpError> {
        let target = self.required_target().await?;
        let result = target
            .engine
            .r#type(&params.reference, &params.text)
            .await
            .map_err(tool_error)?;
        json_result(ToolEnvelope {
            target: target.label(),
            result,
        })
    }

    #[tool(
        name = "browser_eval",
        description = "Evaluate JavaScript in the live twarp Browser pane."
    )]
    async fn browser_eval(
        &self,
        Parameters(params): Parameters<EvalParams>,
    ) -> Result<CallToolResult, McpError> {
        let target = self.required_target().await?;
        let result = target.engine.eval(&params.js).await.map_err(tool_error)?;
        json_result(ToolEnvelope {
            target: target.label(),
            result,
        })
    }

    #[tool(
        name = "browser_screenshot",
        description = "Return a PNG screenshot of the current Browser viewport."
    )]
    async fn browser_screenshot(&self) -> Result<CallToolResult, McpError> {
        let target = self.required_target().await?;
        let png = target.engine.screenshot().await.map_err(tool_error)?;
        Ok(CallToolResult::success(vec![
            Content::text(format!("target: {}", target.label())),
            Content::image(BASE64.encode(png), "image/png"),
        ]))
    }

    #[tool(
        name = "browser_console",
        description = "Return recent console messages and uncaught errors from the live Browser pane."
    )]
    async fn browser_console(&self) -> Result<CallToolResult, McpError> {
        let target = self.required_target().await?;
        let entries = target.engine.console().map_err(tool_error)?;
        json_result(ToolEnvelope {
            target: target.label(),
            result: entries,
        })
    }

    #[tool(
        name = "browser_network",
        description = "Return best-effort fetch/XMLHttpRequest network activity from the live Browser pane."
    )]
    async fn browser_network(&self) -> Result<CallToolResult, McpError> {
        let target = self.required_target().await?;
        let capture = target.engine.network().map_err(tool_error)?;
        json_result(ToolEnvelope {
            target: target.label(),
            result: capture,
        })
    }

    #[tool(
        name = "browser_wait",
        description = "Wait for a selector, navigation settle, or timeout before returning."
    )]
    async fn browser_wait(
        &self,
        Parameters(params): Parameters<WaitParams>,
    ) -> Result<CallToolResult, McpError> {
        let target = self.required_target().await?;
        let timeout = Duration::from_millis(params.timeout_ms.unwrap_or(5_000));
        let wait_spec = if let Some(selector) = params.selector {
            WaitSpec::Selector { selector, timeout }
        } else if params.navigation.unwrap_or(false) {
            WaitSpec::NavigationSettled { timeout }
        } else {
            WaitSpec::Timeout(timeout)
        };

        target.engine.wait(wait_spec).await.map_err(tool_error)?;
        json_result(json!({
            "target": target.label(),
            "result": "ok"
        }))
    }

    async fn resolve_target(&self) -> Result<Option<BrowserAutomationTarget>, McpError> {
        self.spawner
            .spawn(|bridge, ctx| bridge.resolve_target(ctx))
            .await
            .map_err(|_| McpError::internal_error("Browser MCP bridge is unavailable", None))
    }

    async fn required_target(&self) -> Result<BrowserAutomationTarget, McpError> {
        self.resolve_target().await?.ok_or_else(|| {
            McpError::invalid_request(
                "No Browser pane is open. Call browser_navigate first to open one.",
                None,
            )
        })
    }

    async fn open_browser_pane(&self, url: String) -> Result<(), McpError> {
        self.spawner
            .spawn(move |bridge, ctx| bridge.open_browser_pane(url, ctx))
            .await
            .map_err(|_| McpError::internal_error("Browser MCP bridge is unavailable", None))?
            .map_err(|message| McpError::internal_error(message, None))
    }
}

fn json_result<T: Serialize>(value: T) -> Result<CallToolResult, McpError> {
    let value = serde_json::to_value(value).map_err(|err| {
        McpError::internal_error(
            "Failed to serialize browser tool result",
            Some(json!({ "error": err.to_string() })),
        )
    })?;
    let text = serde_json::to_string_pretty(&value).map_err(|err| {
        McpError::internal_error(
            "Failed to render browser tool result",
            Some(json!({ "error": err.to_string() })),
        )
    })?;
    let mut result = CallToolResult::success(vec![Content::text(text)]);
    result.structured_content = Some(value);
    Ok(result)
}

fn tool_error(err: browser::BrowserError) -> McpError {
    McpError::internal_error(err.to_string(), None)
}
