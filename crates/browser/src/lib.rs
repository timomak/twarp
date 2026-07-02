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

    pub async fn navigate(&self, url: &str) -> Result<()> {
        self.load_url(url);
        self.wait(WaitSpec::NavigationSettled {
            timeout: NAVIGATION_SETTLE_TIMEOUT,
        })
        .await
    }

    pub async fn snapshot(&self) -> Result<Snapshot> {
        self.call_automation("snapshot", Vec::new()).await
    }

    pub async fn click(&self, reference: &str) -> Result<ActionResult> {
        self.call_automation("click", vec![json!(reference)]).await
    }

    pub async fn r#type(&self, reference: &str, text: &str) -> Result<ActionResult> {
        self.call_automation("type", vec![json!(reference), json!(text)])
            .await
    }

    pub async fn eval(&self, javascript: &str) -> Result<Value> {
        self.call_automation("eval", vec![json!(javascript)]).await
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
            let found: bool = self
                .call_automation("hasSelector", vec![json!(selector)])
                .await?;
            if found {
                return Ok(());
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

    window.__twarpBrowserAutomation = {
        snapshot() {
            refs = new Map();
            refCounter = 1;
            const elements = Array.from(document.querySelectorAll("a,button,input,textarea,select,summary,[role],[tabindex],[contenteditable=true],[onclick]"))
                .filter(isCandidate)
                .slice(0, 500)
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
            return {
                url: location.href || null,
                title: document.title || "",
                text: (document.body ? document.body.innerText || "" : "").slice(0, 20000),
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

        async eval(source) {
            return sanitize(await (0, eval)(source));
        },

        hasSelector(selector) {
            return document.querySelector(selector) !== null;
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
