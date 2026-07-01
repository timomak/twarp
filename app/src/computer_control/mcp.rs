use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{mpsc, Arc, Mutex},
    time::Duration,
};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use computer_use::{
    Action, MouseButton, Options, Screenshot, ScreenshotParams, ScreenshotRegion, ScrollDirection,
    ScrollDistance, Vector2I,
};
use once_cell::sync::Lazy;
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Content, Implementation, ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio_util::sync::CancellationToken;
use warpui::{Entity, ModelContext, SingletonEntity};

const SERVER_NAME: &str = "twarp-computer-control";
const SSE_PATH: &str = "/sse";
const POST_PATH: &str = "/message";
const DEFAULT_MAX_LONG_EDGE_PX: usize = 1280;
const DEFAULT_MAX_TOTAL_PX: usize = 1_000_000;
const MAX_WAIT_MS: u64 = 10_000;
const MAX_TYPED_TEXT_CHARS: usize = 4_000;
const MAX_SCROLL_DISTANCE: i32 = 20_000;

static AGENT_STATE: Lazy<Arc<Mutex<AgentState>>> =
    Lazy::new(|| Arc::new(Mutex::new(AgentState::default())));
static ACTOR: Lazy<tokio::sync::Mutex<Option<Box<dyn computer_use::Actor>>>> =
    Lazy::new(|| tokio::sync::Mutex::new(None));

#[derive(Default)]
struct AgentState {
    active_session_label: Option<String>,
    latest_status: String,
    latest_capture: Option<CaptureBounds>,
}

#[derive(Clone, Copy, Debug, Serialize)]
struct CaptureBounds {
    origin_x: i32,
    origin_y: i32,
    image_width: usize,
    image_height: usize,
    screen_width: usize,
    screen_height: usize,
}

pub(crate) fn activate_agent_session(session_label: &str) {
    let mut state = AGENT_STATE.lock().expect("computer-control state poisoned");
    state.active_session_label = Some(session_label.to_owned());
    state.latest_capture = None;
    state.latest_status = "Latest: waiting for Claude".to_owned();
}

pub(crate) fn deactivate_agent_session(session_label: Option<&str>) {
    let mut state = AGENT_STATE.lock().expect("computer-control state poisoned");
    if let Some(label) = session_label {
        if state.active_session_label.as_deref() != Some(label) {
            return;
        }
    }
    state.active_session_label = None;
    state.latest_capture = None;
    state.latest_status = "Latest: stopped".to_owned();
}

pub(crate) fn latest_agent_status() -> String {
    AGENT_STATE
        .lock()
        .expect("computer-control state poisoned")
        .latest_status
        .clone()
}

fn set_status(status: impl Into<String>) {
    AGENT_STATE
        .lock()
        .expect("computer-control state poisoned")
        .latest_status = status.into();
}

fn require_active_session() -> Result<String, McpError> {
    AGENT_STATE
        .lock()
        .expect("computer-control state poisoned")
        .active_session_label
        .clone()
        .ok_or_else(|| {
            McpError::invalid_request(
                "Computer control is not active. The user must start computer control from the Claude pane first.",
                None,
            )
        })
}

fn latest_capture() -> Option<CaptureBounds> {
    AGENT_STATE
        .lock()
        .expect("computer-control state poisoned")
        .latest_capture
}

fn update_capture(bounds: CaptureBounds) {
    AGENT_STATE
        .lock()
        .expect("computer-control state poisoned")
        .latest_capture = Some(bounds);
}

pub(crate) struct ComputerControlMcpBridge {
    server: Option<ComputerControlMcpRuntime>,
}

struct ComputerControlMcpRuntime {
    url: String,
    _runtime: tokio::runtime::Runtime,
    _cancel: CancellationToken,
}

impl ComputerControlMcpBridge {
    pub(crate) fn new(_ctx: &mut ModelContext<Self>) -> Self {
        let server = ComputerControlMcpRuntime::start()
            .inspect_err(|err| log::warn!("Failed to start computer-control MCP server: {err}"))
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
}

impl Entity for ComputerControlMcpBridge {
    type Event = ();
}

impl SingletonEntity for ComputerControlMcpBridge {}

impl ComputerControlMcpRuntime {
    fn start() -> Result<Self, String> {
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
                    log::warn!("computer-control MCP HTTP server stopped with error: {err}");
                }
            });

            let _service_cancel = sse_server.with_service(ComputerControlMcpServer::new);
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
struct ComputerControlMcpServer {
    tool_router: ToolRouter<Self>,
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for ComputerControlMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            server_info: Implementation {
                name: SERVER_NAME.to_owned(),
                title: Some("twarp Computer Control".to_owned()),
                version: env!("CARGO_PKG_VERSION").to_owned(),
                icons: None,
                website_url: None,
            },
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            instructions: Some(
                "Use these tools only after the user starts computer control in twarp. First call computer_screenshot. Coordinates for computer_action are screenshot image pixels by default; twarp maps them to the captured screen or region, including downscaled screenshots."
                    .to_owned(),
            ),
            ..Default::default()
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RegionParams {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ScreenshotToolParams {
    /// Optional capture region in physical screen pixels.
    region: Option<RegionParams>,
    /// Downscale the returned PNG so its longer edge is at most this many pixels.
    max_long_edge_px: Option<usize>,
    /// Downscale the returned PNG so its total pixel count is at most this value.
    max_total_px: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ComputerActionParams {
    /// One of: mouse_move, mouse_down, mouse_up, click, double_click, scroll, type_text, key_down, key_up, wait.
    action: String,
    /// X coordinate. By default this is in the latest screenshot image's pixel space.
    x: Option<i32>,
    /// Y coordinate. By default this is in the latest screenshot image's pixel space.
    y: Option<i32>,
    /// Use "screen" when x/y are already physical screen pixels; otherwise omit or use "image".
    coordinate_space: Option<String>,
    /// Mouse button for click/down/up: left, right, middle, back, forward.
    button: Option<String>,
    /// Text for type_text.
    text: Option<String>,
    /// Key for key_down/key_up. Single characters are accepted; common names include enter, tab, escape, space, delete, arrows, home, end, page_up, page_down.
    key: Option<String>,
    /// Wait duration for wait actions.
    duration_ms: Option<u64>,
    /// Scroll direction: up, down, left, right.
    direction: Option<String>,
    /// Scroll distance. Defaults to clicks for small values.
    distance: Option<i32>,
    /// Scroll distance unit: clicks or pixels.
    distance_unit: Option<String>,
    /// Optional screenshot params for the screenshot returned after the action.
    screenshot: Option<ScreenshotToolParams>,
}

#[derive(Debug, Serialize)]
struct ScreenshotMetadata {
    session: String,
    image_width: usize,
    image_height: usize,
    screen_origin_x: i32,
    screen_origin_y: i32,
    screen_width: usize,
    screen_height: usize,
    coordinate_note: &'static str,
    cursor_x: Option<i32>,
    cursor_y: Option<i32>,
}

#[tool_router(router = tool_router)]
impl ComputerControlMcpServer {
    fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        name = "computer_screenshot",
        description = "Capture the user's active display or a physical-pixel region. Returns a PNG plus metadata. Call before computer_action."
    )]
    async fn computer_screenshot(
        &self,
        Parameters(params): Parameters<ScreenshotToolParams>,
    ) -> Result<CallToolResult, McpError> {
        let session = require_active_session()?;
        let screenshot_params = params.screenshot_params()?;
        set_status("Latest: taking screenshot");
        let result = perform_actor_actions(Vec::new(), screenshot_params).await?;
        let screenshot = result
            .screenshot
            .ok_or_else(|| McpError::internal_error("computer_use returned no screenshot", None))?;
        let bounds = capture_bounds(&screenshot, screenshot_params);
        update_capture(bounds);
        set_status(format!(
            "Latest: screenshot {}x{}",
            screenshot.width, screenshot.height
        ));
        screenshot_result(session, screenshot, bounds, result.cursor_position)
    }

    #[tool(
        name = "computer_action",
        description = "Execute one validated mouse/keyboard/wait action, then return a fresh screenshot. Coordinates are latest screenshot image pixels unless coordinate_space is screen."
    )]
    async fn computer_action(
        &self,
        Parameters(params): Parameters<ComputerActionParams>,
    ) -> Result<CallToolResult, McpError> {
        let session = require_active_session()?;
        let screenshot_params = params
            .screenshot
            .as_ref()
            .map(ScreenshotToolParams::screenshot_params)
            .transpose()?
            .unwrap_or_else(default_screenshot_params);
        let summary = action_summary(&params);
        set_status(format!("Latest: proposed {summary}"));
        let actions = validate_action(&params)?;
        require_active_session()?;
        set_status(format!("Latest: executing {summary}"));
        let result = perform_actor_actions(actions, screenshot_params)
            .await
            .inspect_err(|_| {
                set_status(format!("Latest: failed {summary}"));
            })?;
        let screenshot = result.screenshot.ok_or_else(|| {
            set_status(format!("Latest: failed {summary}"));
            McpError::internal_error("computer_use returned no screenshot after action", None)
        })?;
        let bounds = capture_bounds(&screenshot, screenshot_params);
        update_capture(bounds);
        set_status(format!("Latest: executed {summary}"));
        screenshot_result(session, screenshot, bounds, result.cursor_position)
    }
}

impl ScreenshotToolParams {
    fn screenshot_params(&self) -> Result<ScreenshotParams, McpError> {
        let region = self
            .region
            .as_ref()
            .map(RegionParams::screenshot_region)
            .transpose()?;
        Ok(ScreenshotParams {
            max_long_edge_px: Some(self.max_long_edge_px.unwrap_or(DEFAULT_MAX_LONG_EDGE_PX)),
            max_total_px: Some(self.max_total_px.unwrap_or(DEFAULT_MAX_TOTAL_PX)),
            region,
        })
    }
}

impl RegionParams {
    fn screenshot_region(&self) -> Result<ScreenshotRegion, McpError> {
        let bottom_right_x = self
            .x
            .checked_add(self.width)
            .ok_or_else(|| invalid_params("screenshot region x + width overflows"))?;
        let bottom_right_y = self
            .y
            .checked_add(self.height)
            .ok_or_else(|| invalid_params("screenshot region y + height overflows"))?;
        let region = ScreenshotRegion {
            top_left: Vector2I::new(self.x, self.y),
            bottom_right: Vector2I::new(bottom_right_x, bottom_right_y),
        };
        region.validate().map_err(invalid_params)?;
        Ok(region)
    }
}

async fn perform_actor_actions(
    actions: Vec<Action>,
    screenshot_params: ScreenshotParams,
) -> Result<computer_use::ActionResult, McpError> {
    let mut actor = ACTOR.lock().await;
    if actor.is_none() {
        *actor = Some(computer_use::create_actor());
    }
    actor
        .as_mut()
        .expect("actor initialized above")
        .perform_actions(
            &actions,
            Options {
                screenshot_params: Some(screenshot_params),
            },
        )
        .await
        .map_err(|error| McpError::internal_error(error, None))
}

fn default_screenshot_params() -> ScreenshotParams {
    ScreenshotParams {
        max_long_edge_px: Some(DEFAULT_MAX_LONG_EDGE_PX),
        max_total_px: Some(DEFAULT_MAX_TOTAL_PX),
        region: None,
    }
}

fn capture_bounds(screenshot: &Screenshot, params: ScreenshotParams) -> CaptureBounds {
    let (origin_x, origin_y) = params
        .region
        .map(|region| (region.top_left.x(), region.top_left.y()))
        .unwrap_or((0, 0));
    CaptureBounds {
        origin_x,
        origin_y,
        image_width: screenshot.width,
        image_height: screenshot.height,
        screen_width: screenshot.original_width,
        screen_height: screenshot.original_height,
    }
}

fn screenshot_result(
    session: String,
    screenshot: Screenshot,
    bounds: CaptureBounds,
    cursor_position: Option<Vector2I>,
) -> Result<CallToolResult, McpError> {
    let metadata = ScreenshotMetadata {
        session,
        image_width: bounds.image_width,
        image_height: bounds.image_height,
        screen_origin_x: bounds.origin_x,
        screen_origin_y: bounds.origin_y,
        screen_width: bounds.screen_width,
        screen_height: bounds.screen_height,
        coordinate_note:
            "Use image pixel coordinates for computer_action by default; set coordinate_space=screen only for physical screen pixels.",
        cursor_x: cursor_position.map(|p| p.x()),
        cursor_y: cursor_position.map(|p| p.y()),
    };
    let metadata_value = serde_json::to_value(&metadata).map_err(|err| {
        McpError::internal_error(
            "Failed to serialize computer screenshot metadata",
            Some(json!({ "error": err.to_string() })),
        )
    })?;
    let metadata_text = serde_json::to_string_pretty(&metadata_value).map_err(|err| {
        McpError::internal_error(
            "Failed to render computer screenshot metadata",
            Some(json!({ "error": err.to_string() })),
        )
    })?;
    let mut result = CallToolResult::success(vec![
        Content::text(metadata_text),
        Content::image(
            BASE64.encode(screenshot.data),
            screenshot.mime_type.into_owned(),
        ),
    ]);
    result.structured_content = Some(metadata_value);
    Ok(result)
}

fn validate_action(params: &ComputerActionParams) -> Result<Vec<Action>, McpError> {
    let action = normalized(&params.action);
    match action.as_str() {
        "mouse_move" | "move" => {
            let point = validated_point(params)?;
            Ok(vec![Action::MouseMove { to: point }])
        }
        "mouse_down" => {
            let point = validated_point(params)?;
            Ok(vec![Action::MouseDown {
                button: mouse_button(params.button.as_deref())?,
                at: point,
            }])
        }
        "mouse_up" => Ok(vec![Action::MouseUp {
            button: mouse_button(params.button.as_deref())?,
        }]),
        "click" | "left_click" | "right_click" | "middle_click" => {
            let point = validated_point(params)?;
            let button = if action == "left_click" {
                MouseButton::Left
            } else if action == "right_click" {
                MouseButton::Right
            } else if action == "middle_click" {
                MouseButton::Middle
            } else {
                mouse_button(params.button.as_deref())?
            };
            Ok(vec![
                Action::MouseDown {
                    button: button.clone(),
                    at: point,
                },
                Action::MouseUp { button },
            ])
        }
        "double_click" => {
            let point = validated_point(params)?;
            let button = mouse_button(params.button.as_deref())?;
            Ok(vec![
                Action::MouseDown {
                    button: button.clone(),
                    at: point,
                },
                Action::MouseUp {
                    button: button.clone(),
                },
                Action::Wait(Duration::from_millis(80)),
                Action::MouseDown {
                    button: button.clone(),
                    at: point,
                },
                Action::MouseUp { button },
            ])
        }
        "scroll" | "mouse_wheel" => {
            let point = validated_point(params)?;
            Ok(vec![Action::MouseWheel {
                at: point,
                direction: scroll_direction(params.direction.as_deref())?,
                distance: scroll_distance(params)?,
            }])
        }
        "type" | "type_text" => {
            let text = params
                .text
                .clone()
                .ok_or_else(|| invalid_params("type_text requires text"))?;
            if text.chars().count() > MAX_TYPED_TEXT_CHARS {
                return Err(invalid_params(format!(
                    "type_text is limited to {MAX_TYPED_TEXT_CHARS} characters"
                )));
            }
            Ok(vec![Action::TypeText { text }])
        }
        "key_down" => Ok(vec![Action::KeyDown {
            key: key(params.key.as_deref())?,
        }]),
        "key_up" => Ok(vec![Action::KeyUp {
            key: key(params.key.as_deref())?,
        }]),
        "wait" => {
            let duration_ms = params.duration_ms.unwrap_or(1000);
            if duration_ms > MAX_WAIT_MS {
                return Err(invalid_params(format!(
                    "wait is limited to {MAX_WAIT_MS}ms"
                )));
            }
            Ok(vec![Action::Wait(Duration::from_millis(duration_ms))])
        }
        other => Err(invalid_params(format!(
            "unsupported computer action `{other}`"
        ))),
    }
}

fn validated_point(params: &ComputerActionParams) -> Result<Vector2I, McpError> {
    let x = params
        .x
        .ok_or_else(|| invalid_params("action requires x coordinate"))?;
    let y = params
        .y
        .ok_or_else(|| invalid_params("action requires y coordinate"))?;
    let Some(bounds) = latest_capture() else {
        return Err(invalid_params(
            "No screenshot bounds are available. Call computer_screenshot before coordinate actions.",
        ));
    };
    let coordinate_space = params
        .coordinate_space
        .as_deref()
        .map(normalized)
        .unwrap_or_else(|| "image".to_owned());

    if coordinate_space == "screen" || coordinate_space == "physical" {
        let right = bounds
            .origin_x
            .saturating_add(usize_to_i32(bounds.screen_width)?);
        let bottom = bounds
            .origin_y
            .saturating_add(usize_to_i32(bounds.screen_height)?);
        if x < bounds.origin_x || y < bounds.origin_y || x >= right || y >= bottom {
            return Err(invalid_params(format!(
                "screen coordinate ({x}, {y}) is outside captured bounds ({}, {})-({}, {})",
                bounds.origin_x, bounds.origin_y, right, bottom
            )));
        }
        return Ok(Vector2I::new(x, y));
    }

    if coordinate_space != "image" && coordinate_space != "screenshot" {
        return Err(invalid_params(
            "coordinate_space must be `image` or `screen`",
        ));
    }
    if x < 0 || y < 0 || x as usize >= bounds.image_width || y as usize >= bounds.image_height {
        return Err(invalid_params(format!(
            "image coordinate ({x}, {y}) is outside latest screenshot {}x{}",
            bounds.image_width, bounds.image_height
        )));
    }

    let screen_x = bounds.origin_x + scale_coordinate(x, bounds.image_width, bounds.screen_width)?;
    let screen_y =
        bounds.origin_y + scale_coordinate(y, bounds.image_height, bounds.screen_height)?;
    Ok(Vector2I::new(screen_x, screen_y))
}

fn scale_coordinate(
    value: i32,
    image_extent: usize,
    screen_extent: usize,
) -> Result<i32, McpError> {
    if image_extent == 0 {
        return Err(invalid_params(
            "latest screenshot has zero-sized image extent",
        ));
    }
    let scaled = (value as i64)
        .saturating_mul(screen_extent as i64)
        .checked_div(image_extent as i64)
        .ok_or_else(|| invalid_params("failed to scale screenshot coordinate"))?;
    i32::try_from(scaled).map_err(|_| invalid_params("scaled coordinate overflows i32"))
}

fn usize_to_i32(value: usize) -> Result<i32, McpError> {
    i32::try_from(value).map_err(|_| invalid_params("captured bounds exceed i32 coordinates"))
}

fn mouse_button(value: Option<&str>) -> Result<MouseButton, McpError> {
    match value.map(normalized).as_deref().unwrap_or("left") {
        "left" => Ok(MouseButton::Left),
        "right" => Ok(MouseButton::Right),
        "middle" => Ok(MouseButton::Middle),
        "back" => Ok(MouseButton::Back),
        "forward" => Ok(MouseButton::Forward),
        other => Err(invalid_params(format!(
            "unsupported mouse button `{other}`"
        ))),
    }
}

fn scroll_direction(value: Option<&str>) -> Result<ScrollDirection, McpError> {
    match value.map(normalized).as_deref() {
        Some("up") => Ok(ScrollDirection::Up),
        Some("down") | None => Ok(ScrollDirection::Down),
        Some("left") => Ok(ScrollDirection::Left),
        Some("right") => Ok(ScrollDirection::Right),
        Some(other) => Err(invalid_params(format!(
            "unsupported scroll direction `{other}`"
        ))),
    }
}

fn scroll_distance(params: &ComputerActionParams) -> Result<ScrollDistance, McpError> {
    let distance = params.distance.unwrap_or(3);
    let Some(abs_distance) = distance.checked_abs() else {
        return Err(invalid_params("scroll distance is out of range"));
    };
    if distance == 0 || abs_distance > MAX_SCROLL_DISTANCE {
        return Err(invalid_params(format!(
            "scroll distance must be non-zero and at most {MAX_SCROLL_DISTANCE}"
        )));
    }
    match params
        .distance_unit
        .as_deref()
        .map(normalized)
        .as_deref()
        .unwrap_or("clicks")
    {
        "click" | "clicks" => Ok(ScrollDistance::Clicks(distance)),
        "pixel" | "pixels" | "px" => Ok(ScrollDistance::Pixels(distance)),
        other => Err(invalid_params(format!(
            "unsupported scroll distance_unit `{other}`"
        ))),
    }
}

fn key(value: Option<&str>) -> Result<computer_use::Key, McpError> {
    let value = value.ok_or_else(|| invalid_params("key action requires key"))?;
    let mut chars = value.chars();
    if let (Some(ch), None) = (chars.next(), chars.next()) {
        return Ok(computer_use::Key::Char(ch));
    }
    let keycode = match normalized(value).as_str() {
        "enter" | "return" => 36,
        "tab" => 48,
        "space" => 49,
        "escape" | "esc" => 53,
        "delete" | "backspace" => 51,
        "forward_delete" => 117,
        "left" | "arrow_left" => 123,
        "right" | "arrow_right" => 124,
        "down" | "arrow_down" => 125,
        "up" | "arrow_up" => 126,
        "home" => 115,
        "end" => 119,
        "page_up" => 116,
        "page_down" => 121,
        other => {
            return Err(invalid_params(format!(
                "unsupported key `{other}`; use a single character or a supported key name"
            )));
        }
    };
    Ok(computer_use::Key::Keycode(keycode))
}

fn action_summary(params: &ComputerActionParams) -> String {
    let action = normalized(&params.action);
    match action.as_str() {
        "click" | "left_click" | "right_click" | "middle_click" | "double_click" => {
            format!(
                "{} at {},{}",
                action,
                params.x.map_or("?".to_owned(), |x| x.to_string()),
                params.y.map_or("?".to_owned(), |y| y.to_string())
            )
        }
        "type" | "type_text" => {
            let text = params.text.as_deref().unwrap_or_default();
            let preview: String = text.chars().take(32).collect();
            if text.chars().count() > 32 {
                format!("type_text \"{preview}...\"")
            } else {
                format!("type_text \"{preview}\"")
            }
        }
        "wait" => format!("wait {}ms", params.duration_ms.unwrap_or(1000)),
        _ => action,
    }
}

fn normalized(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace(['-', ' '], "_")
}

fn invalid_params(message: impl Into<String>) -> McpError {
    McpError::invalid_params(message.into(), None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_test_capture() {
        update_capture(CaptureBounds {
            origin_x: 100,
            origin_y: 50,
            image_width: 640,
            image_height: 360,
            screen_width: 1280,
            screen_height: 720,
        });
    }

    #[test]
    fn screenshot_region_rejects_invalid_bounds() {
        let region = RegionParams {
            x: 10,
            y: 10,
            width: 0,
            height: 20,
        };
        assert!(region.screenshot_region().is_err());
    }

    #[test]
    fn image_coordinates_scale_to_captured_screen() {
        set_test_capture();
        let point = validated_point(&ComputerActionParams {
            action: "mouse_move".to_owned(),
            x: Some(320),
            y: Some(180),
            coordinate_space: None,
            button: None,
            text: None,
            key: None,
            duration_ms: None,
            direction: None,
            distance: None,
            distance_unit: None,
            screenshot: None,
        })
        .unwrap();
        assert_eq!(point, Vector2I::new(740, 410));
    }

    #[test]
    fn waits_are_bounded() {
        let err = validate_action(&ComputerActionParams {
            action: "wait".to_owned(),
            x: None,
            y: None,
            coordinate_space: None,
            button: None,
            text: None,
            key: None,
            duration_ms: Some(MAX_WAIT_MS + 1),
            direction: None,
            distance: None,
            distance_unit: None,
            screenshot: None,
        });
        assert!(err.is_err());
    }
}
