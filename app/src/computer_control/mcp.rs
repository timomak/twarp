use std::{
    collections::VecDeque,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{mpsc, Arc, Mutex},
    time::Duration,
};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use computer_use::{
    Action, MouseButton, Options, Screenshot, ScreenshotParams, ScreenshotRegion, ScrollDirection,
    ScrollDistance, Vector2I,
};
use instant::Instant;
use once_cell::sync::Lazy;
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Content, Implementation, ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio_util::sync::CancellationToken;
use twarpui::{Entity, ModelContext, SingletonEntity};

const SERVER_NAME: &str = "twarp-computer-control";
const SSE_PATH: &str = "/sse";
const POST_PATH: &str = "/message";
const DEFAULT_MAX_LONG_EDGE_PX: usize = 1280;
const DEFAULT_MAX_TOTAL_PX: usize = 1_000_000;
const MAX_WAIT_MS: u64 = 10_000;
const MAX_TYPED_TEXT_CHARS: usize = 4_000;
const MAX_SCROLL_DISTANCE: i32 = 20_000;
const ACTION_LOG_CAPACITY: usize = 80;
const CONFIRM_POLL_INTERVAL: Duration = Duration::from_millis(200);
const IDLE_AUTO_STOP_TIMEOUT: Duration = Duration::from_secs(60);

static AGENT_STATE: Lazy<Arc<Mutex<AgentState>>> =
    Lazy::new(|| Arc::new(Mutex::new(AgentState::default())));
static ACTOR: Lazy<tokio::sync::Mutex<Option<Box<dyn computer_use::Actor>>>> =
    Lazy::new(|| tokio::sync::Mutex::new(None));

#[derive(Default)]
struct AgentState {
    active_session_label: Option<String>,
    latest_status: String,
    latest_capture: Option<CaptureBounds>,
    pending_confirmation: Option<PendingConfirmation>,
    next_confirmation_id: u64,
    action_log: VecDeque<ActionLogEntry>,
    held_inputs: HeldInputs,
    last_activity: Option<Instant>,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConfirmationDecision {
    Pending,
    Approved,
    Rejected,
}

#[derive(Clone, Debug)]
struct PendingConfirmation {
    id: u64,
    summary: String,
    decision: ConfirmationDecision,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ActionLogEntry {
    sequence: u64,
    label: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct HeldInputs {
    mouse_buttons: Vec<MouseButton>,
    keys: Vec<computer_use::Key>,
}

impl AgentState {
    fn push_log(&mut self, label: impl Into<String>) {
        let sequence = self
            .action_log
            .back()
            .map_or(1, |entry| entry.sequence.saturating_add(1));
        self.action_log.push_back(ActionLogEntry {
            sequence,
            label: label.into(),
        });
        while self.action_log.len() > ACTION_LOG_CAPACITY {
            self.action_log.pop_front();
        }
    }

    fn mark_activity(&mut self) {
        self.last_activity = Some(Instant::now());
    }

    fn idle_timed_out(&self) -> bool {
        self.active_session_label.is_some()
            && self
                .last_activity
                .is_some_and(|activity| activity.elapsed() >= IDLE_AUTO_STOP_TIMEOUT)
    }

    fn log_text(&self) -> String {
        if self.action_log.is_empty() {
            return "No computer-control actions yet.".to_owned();
        }

        self.action_log
            .iter()
            .map(|entry| format!("{}. {}", entry.sequence, entry.label))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn begin_confirmation(&mut self, summary: String) -> u64 {
        self.next_confirmation_id = self.next_confirmation_id.wrapping_add(1);
        let id = self.next_confirmation_id;
        self.pending_confirmation = Some(PendingConfirmation {
            id,
            summary: summary.clone(),
            decision: ConfirmationDecision::Pending,
        });
        self.latest_status = format!("Latest: awaiting approval for {summary}");
        self.push_log(format!("Proposed action: {summary}"));
        self.mark_activity();
        id
    }

    fn decide_pending_confirmation(&mut self, decision: ConfirmationDecision) -> Option<String> {
        let pending = self.pending_confirmation.as_mut()?;
        if pending.decision != ConfirmationDecision::Pending {
            return None;
        }
        pending.decision = decision;
        let summary = pending.summary.clone();
        self.mark_activity();
        Some(summary)
    }

    fn take_release_actions(&mut self) -> Vec<Action> {
        self.held_inputs.take_release_actions()
    }
}

impl HeldInputs {
    fn apply_confirmed_actions(&mut self, actions: &[Action]) {
        for action in actions {
            match action {
                Action::MouseDown { button, .. } => {
                    if !self.mouse_buttons.contains(button) {
                        self.mouse_buttons.push(button.clone());
                    }
                }
                Action::MouseUp { button } => {
                    self.mouse_buttons.retain(|held| held != button);
                }
                Action::KeyDown { key } => {
                    if !self.keys.contains(key) {
                        self.keys.push(key.clone());
                    }
                }
                Action::KeyUp { key } => {
                    self.keys.retain(|held| held != key);
                }
                Action::Wait(_)
                | Action::MouseMove { .. }
                | Action::MouseWheel { .. }
                | Action::TypeText { .. } => {}
            }
        }
    }

    fn take_release_actions(&mut self) -> Vec<Action> {
        let mut actions = Vec::with_capacity(self.mouse_buttons.len() + self.keys.len());
        for button in self.mouse_buttons.drain(..) {
            actions.push(Action::MouseUp { button });
        }
        for key in self.keys.drain(..) {
            actions.push(Action::KeyUp { key });
        }
        actions
    }
}

pub(crate) fn activate_agent_session(session_label: &str) {
    let mut state = AGENT_STATE.lock().expect("computer-control state poisoned");
    state.active_session_label = Some(session_label.to_owned());
    state.latest_capture = None;
    state.pending_confirmation = None;
    state.action_log.clear();
    state.held_inputs = HeldInputs::default();
    state.mark_activity();
    state.latest_status = "Latest: waiting for Claude".to_owned();
    state.push_log(format!("Started computer control for {session_label}"));
}

pub(crate) fn deactivate_agent_session(session_label: Option<&str>) {
    deactivate_agent_session_with_status(
        session_label,
        "Latest: stopped",
        "Stopped computer control",
    );
}

pub(crate) fn deactivate_agent_session_for_idle_timeout(session_label: Option<&str>) {
    deactivate_agent_session_with_status(
        session_label,
        "Latest: stopped due to idleness",
        "Stopped automatically after idle timeout",
    );
}

fn deactivate_agent_session_with_status(
    session_label: Option<&str>,
    status: &'static str,
    log_entry: &'static str,
) {
    let release_actions = {
        let mut state = AGENT_STATE.lock().expect("computer-control state poisoned");
        if let Some(label) = session_label {
            if state.active_session_label.as_deref() != Some(label) {
                return;
            }
        }
        state.active_session_label = None;
        state.latest_capture = None;
        state.pending_confirmation = None;
        state.latest_status = status.to_owned();
        state.last_activity = None;
        state.push_log(log_entry);
        state.take_release_actions()
    };

    release_held_inputs_best_effort(release_actions);
}

pub(crate) fn approve_pending_action() -> bool {
    let mut state = AGENT_STATE.lock().expect("computer-control state poisoned");
    let Some(summary) = state.decide_pending_confirmation(ConfirmationDecision::Approved) else {
        return false;
    };
    state.latest_status = format!("Latest: approved {summary}");
    state.push_log(format!("Approved action: {summary}"));
    true
}

pub(crate) fn reject_pending_action() -> bool {
    let mut state = AGENT_STATE.lock().expect("computer-control state poisoned");
    let Some(summary) = state.decide_pending_confirmation(ConfirmationDecision::Rejected) else {
        return false;
    };
    state.latest_status = format!("Latest: rejected {summary}");
    state.push_log(format!("Rejected action: {summary}"));
    true
}

pub(crate) fn pending_action_requires_confirmation() -> bool {
    AGENT_STATE
        .lock()
        .expect("computer-control state poisoned")
        .pending_confirmation
        .as_ref()
        .is_some_and(|pending| pending.decision == ConfirmationDecision::Pending)
}

pub(crate) fn latest_action_log() -> String {
    AGENT_STATE
        .lock()
        .expect("computer-control state poisoned")
        .log_text()
}

pub(crate) fn idle_timeout_elapsed() -> bool {
    AGENT_STATE
        .lock()
        .expect("computer-control state poisoned")
        .idle_timed_out()
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

fn record_activity() {
    AGENT_STATE
        .lock()
        .expect("computer-control state poisoned")
        .mark_activity();
}

fn push_log(label: impl Into<String>) {
    AGENT_STATE
        .lock()
        .expect("computer-control state poisoned")
        .push_log(label);
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

async fn confirm_action(summary: &str) -> Result<(), McpError> {
    let confirmation_id = {
        let mut state = AGENT_STATE.lock().expect("computer-control state poisoned");
        if state.active_session_label.is_none() {
            return Err(inactive_session_error());
        }
        state.begin_confirmation(summary.to_owned())
    };

    loop {
        tokio::time::sleep(CONFIRM_POLL_INTERVAL).await;
        let decision = {
            let mut state = AGENT_STATE.lock().expect("computer-control state poisoned");
            if state.active_session_label.is_none() {
                return Err(McpError::invalid_request(
                    "Computer control stopped before the pending action was approved.",
                    None,
                ));
            }
            let Some(pending) = state.pending_confirmation.as_ref() else {
                return Err(McpError::invalid_request(
                    "Computer control confirmation is no longer pending.",
                    None,
                ));
            };
            if pending.id != confirmation_id {
                return Err(McpError::invalid_request(
                    "Computer control received a newer pending action before this one completed.",
                    None,
                ));
            }
            match pending.decision {
                ConfirmationDecision::Pending => None,
                decision => {
                    state.pending_confirmation = None;
                    Some(decision)
                }
            }
        };

        match decision {
            None => {}
            Some(ConfirmationDecision::Approved) => return Ok(()),
            Some(ConfirmationDecision::Rejected) => {
                return Err(McpError::invalid_request(
                    "The user rejected the proposed computer-control action.",
                    None,
                ));
            }
            Some(ConfirmationDecision::Pending) => unreachable!(),
        }
    }
}

fn inactive_session_error() -> McpError {
    McpError::invalid_request(
        "Computer control is not active. The user must start computer control from the Claude pane first.",
        None,
    )
}

fn release_held_inputs_best_effort(actions: Vec<Action>) {
    if actions.is_empty() {
        return;
    }

    std::thread::Builder::new()
        .name("computer-control-release-held-inputs".to_owned())
        .spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
            else {
                log::warn!("Failed to create runtime for computer-control held-input release");
                return;
            };
            runtime.block_on(async move {
                let mut actor = ACTOR.lock().await;
                if actor.is_none() {
                    *actor = Some(computer_use::create_actor());
                }
                if let Some(actor) = actor.as_mut() {
                    if let Err(error) = actor
                        .perform_actions(
                            &actions,
                            Options {
                                screenshot_params: None,
                            },
                        )
                        .await
                    {
                        log::warn!("Failed to release held computer-control input: {error}");
                    }
                }
            });
        })
        .ok();
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
        record_activity();
        let screenshot_params = params.screenshot_params()?;
        set_status("Latest: taking screenshot");
        push_log("Screenshot requested");
        let result = perform_actor_actions(Vec::new(), screenshot_params)
            .await
            .inspect_err(|_| {
                set_status("Latest: screenshot failed");
                push_log("Screenshot failed");
            })?;
        let screenshot = result
            .screenshot
            .ok_or_else(|| McpError::internal_error("computer_use returned no screenshot", None))?;
        let bounds = capture_bounds(&screenshot, screenshot_params);
        update_capture(bounds);
        set_status(format!(
            "Latest: screenshot {}x{}",
            screenshot.width, screenshot.height
        ));
        push_log(format!(
            "Screenshot captured: {}x{}",
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
        record_activity();
        let screenshot_params = params
            .screenshot
            .as_ref()
            .map(ScreenshotToolParams::screenshot_params)
            .transpose()?
            .unwrap_or_else(default_screenshot_params);
        let summary = action_summary(&params);
        set_status(format!("Latest: proposed {summary}"));
        let actions = validate_action(&params).inspect_err(|_| {
            set_status(format!("Latest: rejected malformed {summary}"));
            push_log(format!("Failed proposal: malformed {summary}"));
        })?;
        confirm_action(&summary).await?;
        require_active_session()?;
        set_status(format!("Latest: executing {summary}"));
        push_log(format!("Executing action: {summary}"));
        let result = perform_actor_actions(actions, screenshot_params)
            .await
            .inspect_err(|_| {
                set_status(format!("Latest: failed {summary}"));
                push_log(format!("Failed action: {summary}"));
            })?;
        let screenshot = result.screenshot.ok_or_else(|| {
            set_status(format!("Latest: failed {summary}"));
            push_log(format!("Failed action: {summary}: no screenshot returned"));
            McpError::internal_error("computer_use returned no screenshot after action", None)
        })?;
        let bounds = capture_bounds(&screenshot, screenshot_params);
        update_capture(bounds);
        set_status(format!("Latest: executed {summary}"));
        push_log(format!("Executed action: {summary}"));
        push_log(format!(
            "Screenshot captured: {}x{}",
            screenshot.width, screenshot.height
        ));
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
    let result = actor
        .as_mut()
        .expect("actor initialized above")
        .perform_actions(
            &actions,
            Options {
                screenshot_params: Some(screenshot_params),
            },
        )
        .await
        .map_err(|error| McpError::internal_error(error, None));
    if result.is_ok() && !actions.is_empty() {
        AGENT_STATE
            .lock()
            .expect("computer-control state poisoned")
            .held_inputs
            .apply_confirmed_actions(&actions);
    }
    result
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
        coordinate_note: "Use image pixel coordinates for computer_action by default; set coordinate_space=screen only for physical screen pixels.",
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

    #[test]
    fn action_log_is_bounded_and_ordered() {
        let mut state = AgentState::default();
        for index in 0..(ACTION_LOG_CAPACITY + 5) {
            state.push_log(format!("event {index}"));
        }

        assert_eq!(state.action_log.len(), ACTION_LOG_CAPACITY);
        assert_eq!(state.action_log.front().unwrap().sequence, 6);
        assert_eq!(
            state.action_log.back().unwrap().label,
            format!("event {}", ACTION_LOG_CAPACITY + 4)
        );
    }

    #[test]
    fn confirmation_decision_is_explicit() {
        let mut state = AgentState::default();
        let id = state.begin_confirmation("click at 10,20".to_owned());

        assert_eq!(id, 1);
        assert!(matches!(
            state
                .pending_confirmation
                .as_ref()
                .map(|pending| pending.decision),
            Some(ConfirmationDecision::Pending)
        ));
        assert_eq!(
            state.decide_pending_confirmation(ConfirmationDecision::Approved),
            Some("click at 10,20".to_owned())
        );
        assert_eq!(
            state
                .pending_confirmation
                .as_ref()
                .map(|pending| pending.decision),
            Some(ConfirmationDecision::Approved)
        );
        assert_eq!(
            state.decide_pending_confirmation(ConfirmationDecision::Rejected),
            None
        );
    }

    #[test]
    fn idle_timeout_requires_active_session() {
        let mut state = AgentState::default();
        state.last_activity =
            Some(Instant::now() - IDLE_AUTO_STOP_TIMEOUT - Duration::from_secs(1));
        assert!(!state.idle_timed_out());

        state.active_session_label = Some("Claude".to_owned());
        assert!(state.idle_timed_out());

        state.mark_activity();
        assert!(!state.idle_timed_out());
    }

    #[test]
    fn held_inputs_generate_best_effort_releases() {
        let mut held = HeldInputs::default();
        held.apply_confirmed_actions(&[
            Action::MouseDown {
                button: MouseButton::Left,
                at: Vector2I::new(1, 2),
            },
            Action::KeyDown {
                key: computer_use::Key::Char('a'),
            },
        ]);

        let releases = held.take_release_actions();
        assert!(held.mouse_buttons.is_empty());
        assert!(held.keys.is_empty());
        assert_eq!(
            releases,
            vec![
                Action::MouseUp {
                    button: MouseButton::Left
                },
                Action::KeyUp {
                    key: computer_use::Key::Char('a')
                }
            ]
        );
    }
}
