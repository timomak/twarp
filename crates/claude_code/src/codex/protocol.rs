use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;
use serde_json::{json, Value};

use crate::driver::OutgoingMessage;

pub const CODEX_PROGRAM: &str = "codex";
pub const MIN_CODEX_CLI_VERSION: &str = "0.135.0";

static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

pub fn next_request_id() -> String {
    format!("twarp-{}", NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JsonRpcRequest<T> {
    pub id: String,
    pub method: &'static str,
    pub params: T,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeParams {
    pub client_info: ClientInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<InitializeCapabilities>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientInfo {
    pub name: &'static str,
    pub title: &'static str,
    pub version: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeCapabilities {
    pub experimental_api: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadStartParams<'a> {
    pub cwd: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<&'a str>,
    /// twarp 20b: `config.toml`-style overrides (e.g. `mcp_servers`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<&'a Value>,
    pub sandbox: &'static str,
    pub approval_policy: &'static str,
    pub approvals_reviewer: &'static str,
    pub thread_source: &'static str,
    pub session_start_source: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadResumeParams<'a> {
    pub thread_id: &'a str,
    pub cwd: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<&'a str>,
    /// twarp 20b: `config.toml`-style overrides (e.g. `mcp_servers`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<&'a Value>,
    pub sandbox: &'static str,
    pub approval_policy: &'static str,
    pub approvals_reviewer: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnStartParams<'a> {
    pub thread_id: &'a str,
    pub input: Vec<UserInput<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<&'a str>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub enum UserInput<'a> {
    #[serde(rename = "text", rename_all = "camelCase")]
    Text { text: &'a str },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnInterruptParams<'a> {
    pub thread_id: &'a str,
    pub turn_id: &'a str,
}

pub fn initialize_request(id: String) -> Value {
    json!(JsonRpcRequest {
        id,
        method: "initialize",
        params: InitializeParams {
            client_info: ClientInfo {
                name: "twarp",
                title: "twarp",
                version: env!("CARGO_PKG_VERSION"),
            },
            capabilities: Some(InitializeCapabilities {
                experimental_api: true,
            }),
        },
    })
}

pub fn thread_start_request(
    id: String,
    cwd: &Path,
    model: Option<&str>,
    config: Option<&Value>,
    sandbox: &'static str,
    approval_policy: &'static str,
) -> Value {
    let cwd = cwd.to_string_lossy();
    json!(JsonRpcRequest {
        id,
        method: "thread/start",
        params: ThreadStartParams {
            cwd: cwd.as_ref(),
            model,
            config,
            sandbox,
            approval_policy,
            approvals_reviewer: "user",
            thread_source: "user",
            session_start_source: "startup",
        },
    })
}

pub fn thread_resume_request(
    id: String,
    thread_id: &str,
    cwd: &Path,
    model: Option<&str>,
    config: Option<&Value>,
    sandbox: &'static str,
    approval_policy: &'static str,
) -> Value {
    let cwd = cwd.to_string_lossy();
    json!(JsonRpcRequest {
        id,
        method: "thread/resume",
        params: ThreadResumeParams {
            thread_id,
            cwd: cwd.as_ref(),
            model,
            config,
            sandbox,
            approval_policy,
            approvals_reviewer: "user",
        },
    })
}

pub fn turn_start_request(
    id: String,
    thread_id: &str,
    message: &OutgoingMessage,
    model: Option<&str>,
    effort: Option<&str>,
) -> Value {
    json!(JsonRpcRequest {
        id,
        method: "turn/start",
        params: TurnStartParams {
            thread_id,
            input: vec![UserInput::Text {
                text: message.text.as_str(),
            }],
            model,
            effort,
        },
    })
}

pub fn turn_interrupt_request(id: String, thread_id: &str, turn_id: &str) -> Value {
    json!(JsonRpcRequest {
        id,
        method: "turn/interrupt",
        params: TurnInterruptParams { thread_id, turn_id },
    })
}
