//! twarp 26e: shared streamable-HTTP hosting for the built-in MCP servers.
//!
//! Codex's app-server `mcp_servers.<name> = { url }` config speaks the MCP
//! *streamable HTTP* transport, while the built-in servers' existing
//! endpoints (`browser_mcp.rs`, `computer_control/mcp.rs`,
//! `sessions_mcp/bridge.rs`) serve the legacy SSE transport (`/sse` +
//! `/message`) that headless `claude` consumes. Rather than proxy, each
//! bridge additionally hosts its service over rmcp's
//! [`StreamableHttpService`] on its own localhost ephemeral port — same
//! per-session scoping, different wire protocol — and hands *that* URL to
//! Codex.

use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::mpsc,
    time::Duration,
};

use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpService,
};
use tokio_util::sync::CancellationToken;

/// The single MCP endpoint path on a streamable-HTTP server.
pub(crate) const HTTP_PATH: &str = "/mcp";

/// Binds a localhost streamable-HTTP MCP server on an ephemeral port and
/// returns its URL. Mirrors the bridges' SSE `start_server` shape: the
/// server lives on `handle`'s runtime until `cancel` fires, and
/// `make_service` runs once per MCP session — so per-connection state (e.g.
/// the browser bridge's bound pane) behaves exactly like one SSE connection.
pub(crate) fn start_streamable_http_server<S>(
    handle: &tokio::runtime::Handle,
    cancel: CancellationToken,
    server_label: &'static str,
    make_service: impl Fn() -> S + Send + Sync + 'static,
) -> Result<String, String>
where
    S: rmcp::Service<rmcp::RoleServer> + Send + 'static,
{
    let (addr_tx, addr_rx) = mpsc::channel();

    handle.spawn(async move {
        let service = StreamableHttpService::new(
            move || Ok(make_service()),
            LocalSessionManager::default().into(),
            Default::default(),
        );
        let router = axum::Router::new().nest_service(HTTP_PATH, service);
        let bind = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
        let listener = match tokio::net::TcpListener::bind(bind).await {
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

        let shutdown = cancel.clone();
        let server = axum::serve(listener, router).with_graceful_shutdown(async move {
            shutdown.cancelled().await;
        });
        if let Err(err) = server.await {
            log::warn!("{server_label} streamable-HTTP MCP server stopped with error: {err}");
        }
    });

    let addr = addr_rx
        .recv_timeout(Duration::from_secs(2))
        .map_err(|err| err.to_string())??;
    Ok(format!("http://{addr}{HTTP_PATH}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A do-nothing MCP server: `ServerHandler`'s defaults are enough for an
    /// `initialize` round-trip, which is all this transport test needs.
    #[derive(Clone)]
    struct NullServer;

    impl rmcp::ServerHandler for NullServer {}

    /// The helper's endpoint must complete a real MCP initialize handshake
    /// over streamable HTTP — the transport Codex's `{ url }` config speaks.
    #[test]
    fn streamable_http_server_answers_initialize() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        let cancel = CancellationToken::new();
        let url =
            start_streamable_http_server(runtime.handle(), cancel.clone(), "test", || NullServer)
                .unwrap();
        assert!(url.starts_with("http://127.0.0.1:"), "{url}");
        assert!(url.ends_with(HTTP_PATH), "{url}");

        runtime.block_on(async move {
            use rmcp::{transport::StreamableHttpClientTransport, ServiceExt as McpServiceExt};
            let transport = StreamableHttpClientTransport::from_uri(url);
            let client = ().serve(transport).await.expect("initialize round-trip");
            let info = client.peer_info().expect("server info");
            assert_eq!(
                info.protocol_version,
                rmcp::model::ProtocolVersion::default()
            );
            client.cancel().await.ok();
        });
        cancel.cancel();
    }
}
