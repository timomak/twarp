//! twarp 26: the `twarp-sessions` built-in MCP server — session and project
//! visibility for agents (26b: the read path — `list_sessions`,
//! `get_transcript`, `list_projects`).
//!
//! Shape cloned from `browser_mcp.rs`: a main-thread singleton bridge, a
//! dedicated tokio runtime hosting rmcp SSE servers on `127.0.0.1:0`, and one
//! per-Claude-session endpoint so tool calls carry the calling session's
//! identity. The data the tools serve comes from [`registry::SessionRegistry`]
//! — a main-thread-published mirror of every live agent pane — so the MCP
//! runtime never touches views.

#[cfg(not(target_family = "wasm"))]
pub(crate) mod events;
#[cfg(not(target_family = "wasm"))]
pub(crate) mod external;
pub(crate) mod registry;
pub(crate) mod spawn;
pub(crate) mod status;
#[cfg(not(target_family = "wasm"))]
pub(crate) mod store;

#[cfg(not(target_family = "wasm"))]
mod bridge;
#[cfg(not(target_family = "wasm"))]
pub(crate) use bridge::SessionsMcpBridge;
pub(crate) use spawn::SpawnOrigin;

/// Default port for the external listener, as a `usize` for the settings
/// macro (the listener itself uses [`external::DEFAULT_EXTERNAL_PORT`]).
/// Lives here (cfg-free) so wasm builds of the settings group compile.
pub(crate) fn external_default_port() -> usize {
    8377
}
