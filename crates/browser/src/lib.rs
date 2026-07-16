mod webauthn;

use std::time::Duration;

use instant::Instant;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;
use twarpui::{r#async::Timer, WindowId};

#[cfg(target_os = "macos")]
use twarpui::platform::mac::{BrowserWebViewId, Window as MacWindow};

#[cfg(not(target_os = "macos"))]
pub type BrowserWebViewId = usize;

const NAVIGATION_SETTLE_TIMEOUT: Duration = Duration::from_secs(10);
const WAIT_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Error)]
pub enum BrowserError {
    #[error("browser webview is unavailable")]
    WebViewUnavailable,
    #[error("browser automation script is not installed")]
    AutomationUnavailable,
    #[error("javascript evaluation failed: {0}")]
    JavaScript(String),
    #[error("automation returned malformed data: {0}")]
    MalformedData(String),
    #[error("timed out waiting for {0}")]
    Timeout(&'static str),
}

pub type Result<T> = std::result::Result<T, BrowserError>;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Snapshot {
    pub url: Option<String>,
    pub title: String,
    pub text: String,
    pub elements: Vec<SnapshotElement>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct SnapshotElement {
    #[serde(rename = "ref")]
    pub reference: String,
    pub tag: String,
    pub role: Option<String>,
    pub name: String,
    pub text: String,
    pub selector: String,
}

/// twarp 14k: server-side filters so agents don't pay for 200-element
/// footers on every snapshot.
#[derive(Clone, Debug, Default)]
pub struct SnapshotOptions {
    /// Restrict elements (and page text) to this CSS-selector subtree.
    pub selector: Option<String>,
    /// Cap on returned elements (default 500).
    pub max_elements: Option<u64>,
}

/// twarp 14l-2: what lives at an annotation click point.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct DescribedElement {
    pub selector: String,
    pub tag: String,
    pub name: String,
    pub text: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ActionResult {
    #[serde(rename = "ref")]
    pub reference: String,
    pub success: bool,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ConsoleEntry {
    pub level: ConsoleLevel,
    pub message: String,
    pub args: Vec<String>,
    pub timestamp_ms: f64,
    pub source: Option<String>,
    pub line: Option<u32>,
    pub column: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ConsoleLevel {
    Debug,
    Info,
    Log,
    Warn,
    Error,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct NetworkCapture {
    pub limitations: &'static str,
    pub entries: Vec<NetworkEntry>,
}

impl NetworkCapture {
    pub const LIMITATIONS: &'static str =
        "Best-effort in-page fetch/XMLHttpRequest capture; misses pre-injection, non-fetch/XHR subresources, WebSocket frames, and full header/timing fidelity.";
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct NetworkEntry {
    pub id: String,
    pub url: String,
    pub method: String,
    pub status: Option<u16>,
    pub ok: Option<bool>,
    pub phase: NetworkPhase,
    pub started_ms: f64,
    pub ended_ms: Option<f64>,
    pub duration_ms: Option<f64>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NetworkPhase {
    Start,
    Finish,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WaitSpec {
    NavigationSettled { timeout: Duration },
    Selector { selector: String, timeout: Duration },
    Timeout(Duration),
}

#[derive(Debug, Deserialize)]
struct AutomationEnvelope<T> {
    ok: bool,
    value: Option<T>,
    error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrowserProfile {
    Default,
    Private,
}

impl BrowserProfile {
    pub fn is_persistent(self) -> bool {
        matches!(self, Self::Default)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::Private => "Private",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct BrowserEngine {
    window_id: WindowId,
    webview_id: BrowserWebViewId,
    profile: BrowserProfile,
}

impl BrowserEngine {
    pub fn new(window_id: WindowId) -> Option<Self> {
        Self::new_with_profile(window_id, BrowserProfile::Default)
    }

    pub fn new_with_profile(window_id: WindowId, profile: BrowserProfile) -> Option<Self> {
        #[cfg(target_os = "macos")]
        {
            webauthn::install();
            let webview_id = MacWindow::create_browser_webview(window_id, profile.is_persistent())?;
            MacWindow::install_browser_webview_automation_script(
                window_id,
                webview_id,
                INJECTED_SCRIPT,
            );
            Some(Self {
                window_id,
                webview_id,
                profile,
            })
        }

        #[cfg(not(target_os = "macos"))]
        {
            let _ = (window_id, profile);
            None
        }
    }

    pub fn window_id(&self) -> WindowId {
        self.window_id
    }

    pub fn webview_id(&self) -> BrowserWebViewId {
        self.webview_id
    }

    pub fn profile(&self) -> BrowserProfile {
        self.profile
    }

    pub fn clear_default_profile_data(window_id: WindowId) {
        #[cfg(target_os = "macos")]
        MacWindow::clear_browser_website_data(window_id);

        #[cfg(not(target_os = "macos"))]
        let _ = window_id;
    }

    pub fn load_url(&self, url: &str) {
        #[cfg(target_os = "macos")]
        MacWindow::load_browser_webview_url(self.window_id, self.webview_id, url);

        #[cfg(not(target_os = "macos"))]
        let _ = url;
    }

    pub fn go_back(&self) {
        #[cfg(target_os = "macos")]
        MacWindow::browser_webview_go_back(self.window_id, self.webview_id);
    }

    pub fn go_forward(&self) {
        #[cfg(target_os = "macos")]
        MacWindow::browser_webview_go_forward(self.window_id, self.webview_id);
    }

    pub fn reload(&self) {
        #[cfg(target_os = "macos")]
        MacWindow::browser_webview_reload(self.window_id, self.webview_id);
    }

    pub fn stop_loading(&self) {
        #[cfg(target_os = "macos")]
        MacWindow::browser_webview_stop_loading(self.window_id, self.webview_id);
    }

    pub fn can_go_back(&self) -> bool {
        #[cfg(target_os = "macos")]
        return MacWindow::browser_webview_can_go_back(self.window_id, self.webview_id);

        #[cfg(not(target_os = "macos"))]
        false
    }

    pub fn can_go_forward(&self) -> bool {
        #[cfg(target_os = "macos")]
        return MacWindow::browser_webview_can_go_forward(self.window_id, self.webview_id);

        #[cfg(not(target_os = "macos"))]
        false
    }

    pub fn is_loading(&self) -> bool {
        #[cfg(target_os = "macos")]
        return MacWindow::browser_webview_is_loading(self.window_id, self.webview_id);

        #[cfg(not(target_os = "macos"))]
        false
    }

    pub fn current_url(&self) -> Option<String> {
        #[cfg(target_os = "macos")]
        return MacWindow::browser_webview_url(self.window_id, self.webview_id);

        #[cfg(not(target_os = "macos"))]
        None
    }

    pub fn title(&self) -> Option<String> {
        #[cfg(target_os = "macos")]
        return MacWindow::browser_webview_title(self.window_id, self.webview_id);

        #[cfg(not(target_os = "macos"))]
        None
    }

    pub fn focus(&self) {
        #[cfg(target_os = "macos")]
        MacWindow::focus_browser_webview(self.window_id, self.webview_id);
    }

    pub fn set_frame(&self, frame: pathfinder_geometry::rect::RectF) {
        #[cfg(target_os = "macos")]
        MacWindow::set_browser_webview_frame(self.window_id, self.webview_id, frame);

        #[cfg(not(target_os = "macos"))]
        let _ = frame;
    }

    pub fn set_hidden(&self, hidden: bool) {
        #[cfg(target_os = "macos")]
        MacWindow::set_browser_webview_hidden(self.window_id, self.webview_id, hidden);

        #[cfg(not(target_os = "macos"))]
        let _ = hidden;
    }

    pub fn destroy(&self) {
        #[cfg(target_os = "macos")]
        MacWindow::destroy_browser_webview(self.window_id, self.webview_id);
    }

    /// twarp 14l: blocks/unblocks direct user input to the page (agent input
    /// lease). Pane chrome is unaffected.
    pub fn set_input_blocked(&self, blocked: bool) {
        #[cfg(target_os = "macos")]
        MacWindow::set_browser_webview_input_blocked(self.window_id, self.webview_id, blocked);

        #[cfg(not(target_os = "macos"))]
        let _ = blocked;
    }

    /// twarp 14l-2: while armed, the next page click is captured (not
    /// delivered to the page) and retrievable via
    /// [`Self::take_annotation_click`].
    pub fn set_annotation_capture(&self, enabled: bool) {
        #[cfg(target_os = "macos")]
        MacWindow::set_browser_webview_annotation_capture(self.window_id, self.webview_id, enabled);

        #[cfg(not(target_os = "macos"))]
        let _ = enabled;
    }

    pub fn take_annotation_click(&self) -> Option<(f64, f64)> {
        #[cfg(target_os = "macos")]
        return MacWindow::take_browser_webview_annotation_click(self.window_id, self.webview_id);

        #[cfg(not(target_os = "macos"))]
        None
    }

    /// twarp 14l-2: describe the element at viewport coordinates — the anchor
    /// for a user annotation.
    pub async fn describe_point(&self, x: f64, y: f64) -> Result<DescribedElement> {
        self.call_automation("describePoint", vec![json!(x), json!(y)])
            .await
    }

    pub async fn navigate(&self, url: &str) -> Result<()> {
        self.load_url(url);
        self.wait(WaitSpec::NavigationSettled {
            timeout: NAVIGATION_SETTLE_TIMEOUT,
        })
        .await
    }

    pub async fn snapshot(&self, options: SnapshotOptions) -> Result<Snapshot> {
        self.call_automation(
            "snapshot",
            vec![json!(options.selector), json!(options.max_elements)],
        )
        .await
    }

    pub async fn click(&self, reference: &str) -> Result<ActionResult> {
        self.call_automation("click", vec![json!(reference)]).await
    }

    pub async fn r#type(&self, reference: &str, text: &str) -> Result<ActionResult> {
        self.call_automation("type", vec![json!(reference), json!(text)])
            .await
    }

    pub async fn hover(&self, reference: &str) -> Result<ActionResult> {
        self.call_automation("hover", vec![json!(reference)]).await
    }

    pub async fn select(&self, reference: &str, value: &str) -> Result<ActionResult> {
        self.call_automation("select", vec![json!(reference), json!(value)])
            .await
    }

    pub async fn scroll(&self, reference: Option<&str>, dx: f64, dy: f64) -> Result<Value> {
        self.call_automation("scroll", vec![json!(reference), json!(dx), json!(dy)])
            .await
    }

    /// Evaluates the JS as an expression directly via the native
    /// callAsyncJavaScript path. Unlike routing through the injected script's
    /// string-`eval` (the pre-14k behavior), native evaluation is not subject
    /// to the page's `unsafe-eval` CSP, so this works on strict-CSP sites.
    pub async fn eval(&self, javascript: &str) -> Result<Value> {
        let source = raw_eval_script(javascript);
        let response = self.evaluate_javascript(&source).await?;
        let envelope: AutomationEnvelope<Value> = serde_json::from_str(&response)
            .map_err(|err| BrowserError::MalformedData(err.to_string()))?;
        if envelope.ok {
            envelope
                .value
                .ok_or_else(|| BrowserError::MalformedData("missing value".to_owned()))
        } else {
            Err(BrowserError::JavaScript(
                envelope
                    .error
                    .unwrap_or_else(|| "unknown eval error".to_owned()),
            ))
        }
    }

    pub async fn screenshot(&self) -> Result<Vec<u8>> {
        #[cfg(target_os = "macos")]
        return MacWindow::browser_webview_screenshot(self.window_id, self.webview_id)
            .await
            .map_err(BrowserError::JavaScript);

        #[cfg(not(target_os = "macos"))]
        Err(BrowserError::WebViewUnavailable)
    }

    pub fn console(&self) -> Result<Vec<ConsoleEntry>> {
        #[cfg(target_os = "macos")]
        {
            let entries =
                MacWindow::browser_webview_console_entries(self.window_id, self.webview_id);
            parse_entries(&entries)
        }

        #[cfg(not(target_os = "macos"))]
        Err(BrowserError::WebViewUnavailable)
    }

    pub fn network(&self) -> Result<NetworkCapture> {
        #[cfg(target_os = "macos")]
        {
            let entries =
                MacWindow::browser_webview_network_entries(self.window_id, self.webview_id);
            Ok(NetworkCapture {
                limitations: NetworkCapture::LIMITATIONS,
                entries: parse_entries(&entries)?,
            })
        }

        #[cfg(not(target_os = "macos"))]
        Err(BrowserError::WebViewUnavailable)
    }

    pub async fn wait(&self, spec: WaitSpec) -> Result<()> {
        match spec {
            WaitSpec::NavigationSettled { timeout } => {
                self.wait_for_navigation_settled(timeout).await
            }
            WaitSpec::Selector { selector, timeout } => {
                self.wait_for_selector(&selector, timeout).await
            }
            WaitSpec::Timeout(duration) => {
                Timer::after(duration).await;
                Ok(())
            }
        }
    }

    async fn wait_for_navigation_settled(&self, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        let mut observed_loading = false;
        loop {
            let loading = self.is_loading();
            observed_loading |= loading;
            if observed_loading && !loading {
                return Ok(());
            }
            if !observed_loading && self.current_url().is_some() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(BrowserError::Timeout("navigation to settle"));
            }
            Timer::after(WAIT_POLL_INTERVAL).await;
        }
    }

    async fn wait_for_selector(&self, selector: &str, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        loop {
            // A page that is still loading hasn't run the injected automation
            // script yet; treat that as "selector not found yet" and keep
            // polling instead of failing the wait.
            match self
                .call_automation::<bool>("hasSelector", vec![json!(selector)])
                .await
            {
                Ok(true) => return Ok(()),
                Ok(false) => {}
                Err(BrowserError::AutomationUnavailable) => {}
                Err(BrowserError::JavaScript(message))
                    if message.contains("__twarpBrowserAutomation") => {}
                Err(err) => return Err(err),
            }
            if Instant::now() >= deadline {
                return Err(BrowserError::Timeout("selector to appear"));
            }
            Timer::after(WAIT_POLL_INTERVAL).await;
        }
    }

    async fn call_automation<T>(&self, method: &str, args: Vec<Value>) -> Result<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        let source = automation_call_script(method, &args)?;
        let response = self.evaluate_javascript(&source).await?;
        let envelope: AutomationEnvelope<T> = serde_json::from_str(&response)
            .map_err(|err| BrowserError::MalformedData(err.to_string()))?;

        if envelope.ok {
            envelope
                .value
                .ok_or_else(|| BrowserError::MalformedData("missing value".to_owned()))
        } else {
            Err(BrowserError::JavaScript(
                envelope
                    .error
                    .unwrap_or_else(|| "unknown automation error".to_owned()),
            ))
        }
    }

    async fn evaluate_javascript(&self, source: &str) -> Result<String> {
        #[cfg(target_os = "macos")]
        return MacWindow::browser_webview_evaluate_javascript(
            self.window_id,
            self.webview_id,
            source,
        )
        .await
        .map_err(|error| {
            if error.contains("__twarpBrowserAutomation") {
                BrowserError::AutomationUnavailable
            } else {
                BrowserError::JavaScript(error)
            }
        });

        #[cfg(not(target_os = "macos"))]
        {
            let _ = source;
            Err(BrowserError::WebViewUnavailable)
        }
    }
}

fn parse_entries<T>(entries_json: &str) -> Result<Vec<T>>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_str(entries_json).map_err(|err| BrowserError::MalformedData(err.to_string()))
}

/// Wraps user JS so it evaluates as an (optionally async) expression and the
/// result comes back as the same ok/value/error envelope the automation
/// methods use. The body is compiled natively by callAsyncJavaScript, so page
/// CSP cannot block it.
fn raw_eval_script(javascript: &str) -> String {
    format!(
        r#"(async function() {{
    try {{
        const raw = await (async () => ( {javascript} ))();
        let value;
        if (raw === undefined) {{
            value = null;
        }} else {{
            try {{
                value = JSON.parse(JSON.stringify(raw) ?? "null");
            }} catch (_) {{
                value = String(raw);
            }}
        }}
        return JSON.stringify({{ ok: true, value }});
    }} catch (error) {{
        return JSON.stringify({{ ok: false, error: String(error && error.message ? error.message : error) }});
    }}
}})()"#
    )
}

fn automation_call_script(method: &str, args: &[Value]) -> Result<String> {
    let method = serde_json::to_string(method)
        .map_err(|err| BrowserError::MalformedData(err.to_string()))?;
    let args =
        serde_json::to_string(args).map_err(|err| BrowserError::MalformedData(err.to_string()))?;
    Ok(format!(
        r#"(async function() {{
    const automation = window.__twarpBrowserAutomation;
    if (!automation || typeof automation[{method}] !== "function") {{
        return JSON.stringify({{ ok: false, error: "__twarpBrowserAutomation method unavailable" }});
    }}
    try {{
        const value = await automation[{method}](...{args});
        return JSON.stringify({{ ok: true, value }});
    }} catch (error) {{
        return JSON.stringify({{ ok: false, error: String(error && error.message ? error.message : error) }});
    }}
}})()"#
    ))
}

const INJECTED_SCRIPT: &str = r##"
(function () {
    if (window.__twarpBrowserAutomationInstalled) {
        return;
    }
    window.__twarpBrowserAutomationInstalled = true;

    const REF_ATTR = "data-twarp-ref";
    let refs = new Map();
    let refCounter = 1;
    let requestCounter = 1;

    function now() {
        return (window.performance && performance.now) ? performance.now() : Date.now();
    }

    function send(message) {
        try {
            if (window.webkit && window.webkit.messageHandlers && window.webkit.messageHandlers.twarpAutomation) {
                window.webkit.messageHandlers.twarpAutomation.postMessage(message);
            }
        } catch (_) {}
    }

    function stringify(value) {
        try {
            if (typeof value === "string") return value;
            return JSON.stringify(value);
        } catch (_) {
            return String(value);
        }
    }

    function sanitize(value) {
        if (value === undefined) return null;
        try {
            return JSON.parse(JSON.stringify(value));
        } catch (_) {
            return String(value);
        }
    }

    function cssEscape(value) {
        if (window.CSS && CSS.escape) return CSS.escape(value);
        return String(value).replace(/[^a-zA-Z0-9_-]/g, "\\$&");
    }

    function selectorFor(element) {
        if (!element || !element.tagName) return "";
        if (element.id) return "#" + cssEscape(element.id);
        const parts = [];
        let current = element;
        while (current && current.nodeType === Node.ELEMENT_NODE && parts.length < 4) {
            let part = current.tagName.toLowerCase();
            if (current.classList && current.classList.length > 0) {
                part += "." + Array.from(current.classList).slice(0, 2).map(cssEscape).join(".");
            }
            const parent = current.parentElement;
            if (parent) {
                const siblings = Array.from(parent.children).filter((child) => child.tagName === current.tagName);
                if (siblings.length > 1) part += `:nth-of-type(${siblings.indexOf(current) + 1})`;
            }
            parts.unshift(part);
            current = parent;
        }
        return parts.join(" > ");
    }

    function accessibleName(element) {
        return element.getAttribute("aria-label") ||
            element.getAttribute("title") ||
            element.getAttribute("alt") ||
            element.getAttribute("placeholder") ||
            element.value ||
            element.innerText ||
            element.textContent ||
            "";
    }

    function isCandidate(element) {
        if (!element || !element.getBoundingClientRect) return false;
        const style = window.getComputedStyle(element);
        if (style.visibility === "hidden" || style.display === "none") return false;
        const rect = element.getBoundingClientRect();
        if (rect.width <= 0 || rect.height <= 0) return false;
        return element.matches("a,button,input,textarea,select,summary,[role],[tabindex],[contenteditable=true],[onclick]");
    }

    function resolveRef(reference) {
        const element = refs.get(reference) || document.querySelector(`[${REF_ATTR}="${cssEscape(reference)}"]`);
        if (!element || !element.isConnected) {
            throw new Error("stale element ref; call snapshot again");
        }
        return element;
    }

    function dispatchMouseClick(element) {
        const rect = element.getBoundingClientRect();
        const x = rect.left + Math.max(1, rect.width / 2);
        const y = rect.top + Math.max(1, rect.height / 2);
        const init = { bubbles: true, cancelable: true, view: window, clientX: x, clientY: y, button: 0 };
        element.dispatchEvent(new MouseEvent("mouseover", init));
        element.dispatchEvent(new MouseEvent("mousemove", init));
        element.dispatchEvent(new MouseEvent("mousedown", init));
        element.focus({ preventScroll: true });
        element.dispatchEvent(new MouseEvent("mouseup", init));
        element.dispatchEvent(new MouseEvent("click", init));
    }

    function setElementText(element, text) {
        element.focus({ preventScroll: true });
        if (element.isContentEditable) {
            element.textContent = text;
            element.dispatchEvent(new InputEvent("input", { bubbles: true, data: text, inputType: "insertText" }));
            return;
        }
        if ("value" in element) {
            const descriptor = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(element), "value");
            if (descriptor && descriptor.set) {
                descriptor.set.call(element, text);
            } else {
                element.value = text;
            }
            element.dispatchEvent(new InputEvent("input", { bubbles: true, data: text, inputType: "insertText" }));
            element.dispatchEvent(new Event("change", { bubbles: true }));
        }
    }

    const originalConsole = {};
    ["debug", "info", "log", "warn", "error"].forEach((level) => {
        originalConsole[level] = console[level];
        console[level] = function (...args) {
            send({
                type: "console",
                level,
                message: args.map(stringify).join(" "),
                args: args.map(stringify),
                timestamp_ms: now(),
                source: null,
                line: null,
                column: null
            });
            return originalConsole[level].apply(console, args);
        };
    });

    window.addEventListener("error", (event) => {
        send({
            type: "console",
            level: "error",
            message: event.message || "Uncaught error",
            args: [event.message || "Uncaught error"],
            timestamp_ms: now(),
            source: event.filename || null,
            line: event.lineno || null,
            column: event.colno || null
        });
    });

    window.addEventListener("unhandledrejection", (event) => {
        send({
            type: "console",
            level: "error",
            message: stringify(event.reason || "Unhandled promise rejection"),
            args: [stringify(event.reason || "Unhandled promise rejection")],
            timestamp_ms: now(),
            source: null,
            line: null,
            column: null
        });
    });

    const originalFetch = window.fetch;
    if (originalFetch) {
        window.fetch = async function (input, init) {
            const id = String(requestCounter++);
            const url = typeof input === "string" ? input : (input && input.url) || "";
            const method = (init && init.method) || (input && input.method) || "GET";
            const started = now();
            send({ type: "network", id, url, method, status: null, ok: null, phase: "start", started_ms: started, ended_ms: null, duration_ms: null, error: null });
            try {
                const response = await originalFetch.apply(this, arguments);
                const ended = now();
                send({ type: "network", id, url: response.url || url, method, status: response.status, ok: response.ok, phase: "finish", started_ms: started, ended_ms: ended, duration_ms: ended - started, error: null });
                return response;
            } catch (error) {
                const ended = now();
                send({ type: "network", id, url, method, status: null, ok: false, phase: "error", started_ms: started, ended_ms: ended, duration_ms: ended - started, error: String(error && error.message ? error.message : error) });
                throw error;
            }
        };
    }

    const originalOpen = XMLHttpRequest.prototype.open;
    const originalSend = XMLHttpRequest.prototype.send;
    XMLHttpRequest.prototype.open = function (method, url) {
        this.__twarpRequest = { method: method || "GET", url: String(url || ""), id: String(requestCounter++), started: 0 };
        return originalOpen.apply(this, arguments);
    };
    XMLHttpRequest.prototype.send = function () {
        const request = this.__twarpRequest || { method: "GET", url: "", id: String(requestCounter++), started: 0 };
        request.started = now();
        send({ type: "network", id: request.id, url: request.url, method: request.method, status: null, ok: null, phase: "start", started_ms: request.started, ended_ms: null, duration_ms: null, error: null });
        this.addEventListener("loadend", () => {
            const ended = now();
            send({ type: "network", id: request.id, url: request.url, method: request.method, status: this.status || null, ok: this.status >= 200 && this.status < 400, phase: "finish", started_ms: request.started, ended_ms: ended, duration_ms: ended - request.started, error: null });
        });
        this.addEventListener("error", () => {
            const ended = now();
            send({ type: "network", id: request.id, url: request.url, method: request.method, status: this.status || null, ok: false, phase: "error", started_ms: request.started, ended_ms: ended, duration_ms: ended - request.started, error: "XMLHttpRequest error" });
        });
        return originalSend.apply(this, arguments);
    };

    // --- WebAuthn (passkey) bridge -------------------------------------
    // Override navigator.credentials.create/get so passkey ceremonies run
    // through twarp's software authenticator (native side does Touch ID +
    // crypto). Existing sites see a standard PublicKeyCredential.
    (function installWebAuthnBridge() {
        if (!window.PublicKeyCredential || window.__twarpWebAuthnInstalled) {
            return;
        }
        window.__twarpWebAuthnInstalled = true;

        const pending = new Map();
        let requestCounter = 1;

        function toB64Url(buffer) {
            const bytes = new Uint8Array(buffer);
            let binary = "";
            for (let i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]);
            return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
        }

        function fromB64Url(value) {
            const padded = String(value).replace(/-/g, "+").replace(/_/g, "/");
            const binary = atob(padded + "===".slice((padded.length + 3) % 4));
            const bytes = new Uint8Array(binary.length);
            for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
            return bytes.buffer;
        }

        function normalizeChallengeOrId(value) {
            // WebAuthn passes BufferSource; the native side wants base64url.
            if (value instanceof ArrayBuffer) return toB64Url(value);
            if (ArrayBuffer.isView(value)) return toB64Url(value.buffer);
            return String(value || "");
        }

        function post(kind, detail) {
            const id = "wa" + requestCounter++;
            const message = Object.assign({ type: "webauthn", id, kind, origin: location.origin }, detail);
            return new Promise((resolve, reject) => {
                pending.set(id, { resolve, reject });
                try {
                    window.webkit.messageHandlers.twarpAutomation.postMessage(message);
                } catch (err) {
                    pending.delete(id);
                    reject(new DOMException("Passkey bridge unavailable", "NotAllowedError"));
                }
            });
        }

        // Called by native code with the ceremony result.
        window.__twarpWebAuthnComplete = function (requestId, response) {
            const entry = pending.get(requestId);
            if (!entry) return;
            pending.delete(requestId);
            if (!response || !response.ok) {
                const error = (response && response.error) || {};
                entry.reject(new DOMException(error.message || "Passkey request failed", error.name || "NotAllowedError"));
                return;
            }
            entry.resolve(response.credential);
        };

        function buildPublicKeyCredential(cred, extraResponse) {
            const rawId = fromB64Url(cred.id);
            const response = Object.assign({
                clientDataJSON: fromB64Url(cred.clientDataJSON),
            }, extraResponse);
            const credential = {
                id: cred.id,
                rawId,
                type: "public-key",
                authenticatorAttachment: "platform",
                response,
                getClientExtensionResults() { return {}; },
            };
            Object.setPrototypeOf(credential, window.PublicKeyCredential.prototype);
            return credential;
        }

        const originalCreate = navigator.credentials.create.bind(navigator.credentials);
        const originalGet = navigator.credentials.get.bind(navigator.credentials);

        navigator.credentials.create = async function (options) {
            if (!options || !options.publicKey) return originalCreate(options);
            const pk = options.publicKey;
            const cred = await post("create", {
                rpId: (pk.rp && pk.rp.id) || location.hostname,
                challenge: normalizeChallengeOrId(pk.challenge),
                userId: pk.user ? normalizeChallengeOrId(pk.user.id) : "",
                userName: (pk.user && (pk.user.name || pk.user.displayName)) || "",
                algs: (pk.pubKeyCredParams || []).map((p) => p.alg),
                excludeIds: (pk.excludeCredentials || []).map((c) => normalizeChallengeOrId(c.id)),
            });
            return buildPublicKeyCredential(cred, {
                attestationObject: fromB64Url(cred.attestationObject),
                getAuthenticatorData() { return fromB64Url(cred.authData); },
                getPublicKey() { return fromB64Url(cred.publicKey); },
                getPublicKeyAlgorithm() { return cred.publicKeyAlg; },
                getTransports() { return ["internal"]; },
            });
        };

        navigator.credentials.get = async function (options) {
            if (!options || !options.publicKey) return originalGet(options);
            const pk = options.publicKey;
            const cred = await post("get", {
                rpId: pk.rpId || location.hostname,
                challenge: normalizeChallengeOrId(pk.challenge),
                allowIds: (pk.allowCredentials || []).map((c) => normalizeChallengeOrId(c.id)),
            });
            return buildPublicKeyCredential(cred, {
                authenticatorData: fromB64Url(cred.authData),
                signature: fromB64Url(cred.signature),
                userHandle: cred.userHandle ? fromB64Url(cred.userHandle) : null,
                getAuthenticatorData() { return fromB64Url(cred.authData); },
            });
        };

        // Advertise platform-authenticator availability so sites offer passkeys.
        window.PublicKeyCredential.isUserVerifyingPlatformAuthenticatorAvailable = function () {
            return Promise.resolve(true);
        };
        if (window.PublicKeyCredential.isConditionalMediationAvailable) {
            window.PublicKeyCredential.isConditionalMediationAvailable = function () {
                return Promise.resolve(false);
            };
        }
    })();

    window.__twarpBrowserAutomation = {
        snapshot(scopeSelector, maxElements) {
            refs = new Map();
            refCounter = 1;
            let root = document;
            if (scopeSelector) {
                root = document.querySelector(scopeSelector);
                if (!root) {
                    throw new Error("snapshot selector matched nothing: " + scopeSelector);
                }
            }
            const cap = Math.max(1, Math.min(Number(maxElements) || 500, 500));
            const elements = Array.from(root.querySelectorAll("a,button,input,textarea,select,summary,[role],[tabindex],[contenteditable=true],[onclick]"))
                .filter(isCandidate)
                .slice(0, cap)
                .map((element) => {
                    const reference = "e" + refCounter++;
                    refs.set(reference, element);
                    element.setAttribute(REF_ATTR, reference);
                    return {
                        ref: reference,
                        tag: element.tagName.toLowerCase(),
                        role: element.getAttribute("role"),
                        name: accessibleName(element).trim().slice(0, 200),
                        text: (element.innerText || element.textContent || "").trim().slice(0, 500),
                        selector: selectorFor(element)
                    };
                });
            const textRoot = scopeSelector ? root : document.body;
            return {
                url: location.href || null,
                title: document.title || "",
                text: (textRoot ? textRoot.innerText || "" : "").slice(0, 20000),
                elements
            };
        },

        click(reference) {
            const element = resolveRef(reference);
            element.scrollIntoView({ block: "center", inline: "center", behavior: "instant" });
            dispatchMouseClick(element);
            return { ref: reference, success: true, message: "clicked" };
        },

        type(reference, text) {
            const element = resolveRef(reference);
            element.scrollIntoView({ block: "center", inline: "center", behavior: "instant" });
            setElementText(element, String(text));
            return { ref: reference, success: true, message: "typed" };
        },

        hover(reference) {
            const element = resolveRef(reference);
            element.scrollIntoView({ block: "center", inline: "center", behavior: "instant" });
            const rect = element.getBoundingClientRect();
            const init = {
                bubbles: true, cancelable: true, view: window,
                clientX: rect.left + Math.max(1, rect.width / 2),
                clientY: rect.top + Math.max(1, rect.height / 2)
            };
            element.dispatchEvent(new MouseEvent("mouseenter", { ...init, bubbles: false }));
            element.dispatchEvent(new MouseEvent("mouseover", init));
            element.dispatchEvent(new MouseEvent("mousemove", init));
            return { ref: reference, success: true, message: "hovered" };
        },

        select(reference, value) {
            const element = resolveRef(reference);
            if (element.tagName !== "SELECT") {
                throw new Error("select requires a <select> element ref");
            }
            const option = Array.from(element.options).find(
                (opt) => opt.value === value || opt.label === value || opt.text === value
            );
            if (!option) {
                throw new Error("no option matching: " + value);
            }
            element.value = option.value;
            element.dispatchEvent(new Event("input", { bubbles: true }));
            element.dispatchEvent(new Event("change", { bubbles: true }));
            return { ref: reference, success: true, message: "selected " + option.value };
        },

        scroll(reference, dx, dy) {
            const deltaX = Number(dx) || 0;
            const deltaY = Number(dy) || 0;
            if (reference) {
                const element = resolveRef(reference);
                element.scrollBy(deltaX, deltaY);
                return { x: element.scrollLeft, y: element.scrollTop };
            }
            window.scrollBy(deltaX, deltaY);
            return { x: window.scrollX, y: window.scrollY };
        },

        hasSelector(selector) {
            return document.querySelector(selector) !== null;
        },

        describePoint(x, y) {
            const element = document.elementFromPoint(Number(x), Number(y));
            if (!element) {
                throw new Error("no element at point");
            }
            return {
                selector: selectorFor(element),
                tag: element.tagName.toLowerCase(),
                name: accessibleName(element).trim().slice(0, 120),
                text: (element.innerText || element.textContent || "").trim().slice(0, 200)
            };
        }
    };
})();
"##;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_console_entries() {
        let entries: Vec<ConsoleEntry> = parse_entries(
            r#"[{"level":"log","message":"ready","args":["ready"],"timestamp_ms":1,"source":null,"line":null,"column":null}]"#,
        )
        .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].level, ConsoleLevel::Log);
        assert_eq!(entries[0].message, "ready");
    }

    #[test]
    fn parses_network_entries() {
        let entries: Vec<NetworkEntry> = parse_entries(
            r#"[{"id":"1","url":"/api","method":"POST","status":200,"ok":true,"phase":"finish","started_ms":1,"ended_ms":3,"duration_ms":2,"error":null}]"#,
        )
        .unwrap();
        assert_eq!(entries[0].phase, NetworkPhase::Finish);
        assert_eq!(entries[0].status, Some(200));
    }

    #[test]
    fn automation_call_escapes_args() {
        let script = automation_call_script("eval", &[json!("document.title")]).unwrap();
        assert!(script.contains(r#""eval""#));
        assert!(script.contains(r#""document.title""#));
    }

    #[test]
    fn snapshot_refs_deserialize_from_ref_field() {
        let snapshot: Snapshot = serde_json::from_str(
            r##"{"url":"https://example.com","title":"Example","text":"Hello","elements":[{"ref":"e1","tag":"button","role":null,"name":"Submit","text":"Submit","selector":"#submit"}]}"##,
        )
        .unwrap();
        assert_eq!(snapshot.elements[0].reference, "e1");
    }
}
