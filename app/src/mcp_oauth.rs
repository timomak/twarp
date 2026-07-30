//! twarp 24b: connection + browser-consent OAuth for remote MCP servers.
//!
//! Adding a hosted MCP server on the Plugins page is "paste the URL, press
//! Connect". This module is what Connect does:
//!
//! 1. **Probe** the URL with whatever credentials the entry already carries
//!    (static headers, a cached OAuth token). A successful MCP handshake
//!    reports the server's tool count; a 401 is classified into "this server
//!    speaks OAuth" (RFC 9728 / RFC 8414 metadata is discoverable) vs "this
//!    server wants a static credential".
//! 2. **Authorize**, for the OAuth case: dynamic client registration (falling
//!    back to statically-bundled credentials for issuers that don't support
//!    DCR — see [`ChannelState::mcp_oauth_provider_by_issuer`]), PKCE, then
//!    open the provider's consent page in the user's browser.
//! 3. **Complete** when the provider redirects to
//!    `twarp://mcp/oauth2callback?server_id=…&code=…&state=…`, which macOS
//!    routes to [`crate::uri::handle_incoming_uri`]. Tokens land in the OS
//!    keychain under `mcp.oauth.<server id>`, never in the DB or a config
//!    file.
//!
//! Access tokens are also cached in memory, because session spawn
//! ([`crate::mcp_registry::McpRegistryModel::claude_mcp_config_json`]) is
//! synchronous and must not block the main thread on keychain or network I/O.
//! The cache is seeded at startup and kept fresh by a background refresher;
//! a session spawned with no cached token simply gets no `Authorization`
//! header, the provider answers 401, and the user reconnects on the Plugins
//! page.
//!
//! Ported from the pre-ai-removal `app/src/ai/mcp/templatable_manager/
//! oauth.rs` (see `specs/APP-4099/TECH.md`), which drove the same rmcp
//! machinery over the same `twarp://mcp/oauth2callback` redirect.

use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use twarp_core::channel::ChannelState;
use twarpui::{AppContext, Entity, ModelContext, SingletonEntity};

use crate::mcp_registry::{McpAuth, McpRegistryModel, McpServerEntry, McpTransport, OauthTokens};

/// How long a browser-consent flow may stay pending before it gives up and
/// returns the row to a retriable state (PRODUCT.md P6).
const CONSENT_TIMEOUT: Duration = Duration::from_secs(120);

/// How often the background refresher re-checks cached tokens.
#[cfg(not(target_family = "wasm"))]
const REFRESH_INTERVAL: Duration = Duration::from_secs(600);

/// Keychain account for a server's stored OAuth credentials. The service name
/// is the app's data domain, set in [`crate::initialize_app`].
fn keychain_key(server_id: &str) -> String {
    format!("mcp.oauth.{server_id}")
}

/// The redirect the provider sends the user back to. `server_id` rides along
/// so the callback can be routed without consulting the CSRF map first.
fn redirect_uri(server_id: &str) -> String {
    format!(
        "{}://mcp/oauth2callback?server_id={server_id}",
        ChannelState::url_scheme()
    )
}

/// twarp 24b: what the Plugins page shows for one server row.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum McpAuthStatus {
    /// A stdio server — nothing to connect to, so no probe is attempted.
    Local,
    /// A remote server that has never connected, or was disconnected.
    #[default]
    NotConnected,
    /// A probe is in flight.
    Connecting,
    /// The consent page is open in the browser.
    WaitingForBrowser,
    /// Handshake succeeded; `tools` is what the server advertised.
    Connected { tools: usize },
    /// The server wants a credential it can't negotiate for us — the editor
    /// opens Advanced and focuses the Headers field.
    NeedsStaticAuth,
    /// Network failure, refused consent, timeout, or a protocol error. Always
    /// retriable by pressing Connect again.
    Error(String),
    /// A rendering-only wrapper: another status, labelled with the server it
    /// describes. Produced by [`Self::prefixed`] for multi-server cards.
    Named { server: String, label: String },
}

impl McpAuthStatus {
    /// Chip label shown next to the server row.
    pub fn label(&self) -> String {
        match self {
            McpAuthStatus::Local => "Local".to_owned(),
            McpAuthStatus::NotConnected => "Not connected".to_owned(),
            McpAuthStatus::Connecting => "Connecting…".to_owned(),
            McpAuthStatus::WaitingForBrowser => "Waiting for browser…".to_owned(),
            McpAuthStatus::Connected { tools } => match tools {
                1 => "Connected · 1 tool".to_owned(),
                n => format!("Connected · {n} tools"),
            },
            McpAuthStatus::NeedsStaticAuth => "Needs authorization".to_owned(),
            McpAuthStatus::Error(_) => "Error".to_owned(),
            McpAuthStatus::Named { server, label } => format!("{server}: {label}"),
        }
    }

    /// Longer text for the row's hover/expand, when there is any.
    pub fn detail(&self) -> Option<&str> {
        match self {
            McpAuthStatus::Error(message) => Some(message.as_str()),
            McpAuthStatus::NeedsStaticAuth => {
                Some("This server expects an Authorization header.")
            }
            _ => None,
        }
    }

    /// The same status, labelled with the server it belongs to — used when a
    /// plugin card lists more than one remote server.
    pub fn prefixed(&self, server_name: &str) -> Self {
        match self {
            McpAuthStatus::Error(message) => {
                McpAuthStatus::Error(format!("{server_name}: {message}"))
            }
            other => McpAuthStatus::Named {
                server: server_name.to_owned(),
                label: other.label(),
            },
        }
    }

    pub fn is_connected(&self) -> bool {
        matches!(self, McpAuthStatus::Connected { .. })
    }

    /// Whether a Connect press should do anything (a flow is not already in
    /// flight).
    pub fn is_busy(&self) -> bool {
        matches!(
            self,
            McpAuthStatus::Connecting | McpAuthStatus::WaitingForBrowser
        )
    }
}

/// Singleton owning per-server connection state. Registered in
/// [`crate::initialize_app`] after the MCP registry.
pub struct McpOauthModel {
    status: HashMap<String, McpAuthStatus>,
    /// Access tokens by server id — the synchronous read used at spawn time.
    tokens: OauthTokens,
    /// Monotonic per-server flow counter. A callback or timeout carrying a
    /// stale generation is ignored, which is what makes "starting a second
    /// Connect cancels the first" (P13) safe.
    generation: HashMap<String, u64>,
    #[cfg(not(target_family = "wasm"))]
    imp: Option<imp::Runtime>,
}

impl McpOauthModel {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        #[cfg(not(target_family = "wasm"))]
        let imp = imp::Runtime::start(ctx)
            .inspect_err(|err| log::warn!("Failed to start the MCP OAuth runtime: {err}"))
            .ok();
        #[cfg(target_family = "wasm")]
        let _ = ctx;

        Self {
            status: HashMap::new(),
            tokens: OauthTokens::new(),
            generation: HashMap::new(),
            #[cfg(not(target_family = "wasm"))]
            imp,
        }
    }

    /// Cached access tokens for session spawn. Never blocks.
    pub fn access_tokens(&self) -> OauthTokens {
        self.tokens.clone()
    }

    pub fn status(&self, entry: &McpServerEntry) -> McpAuthStatus {
        self.status_for_form(Some(&entry.id), entry.transport)
    }

    /// Status for a Plugins-page sub-form, which may not have a saved id yet
    /// (a brand-new server reads as Not connected until its first Connect).
    pub fn status_for_form(
        &self,
        server_id: Option<&str>,
        transport: McpTransport,
    ) -> McpAuthStatus {
        if transport == McpTransport::Stdio {
            return McpAuthStatus::Local;
        }
        server_id
            .and_then(|id| self.status.get(id))
            .cloned()
            .unwrap_or_default()
    }

    /// Bump `server_id`'s flow generation, invalidating any in-flight
    /// callback or timeout for it, and return the new value.
    fn next_generation(&mut self, server_id: &str) -> u64 {
        let generation = self.generation.entry(server_id.to_owned()).or_default();
        *generation += 1;
        *generation
    }

    fn is_current(&self, server_id: &str, generation: u64) -> bool {
        self.generation.get(server_id).copied().unwrap_or_default() == generation
    }

    fn set_status(&mut self, server_id: &str, status: McpAuthStatus, ctx: &mut ModelContext<Self>) {
        self.status.insert(server_id.to_owned(), status);
        ctx.notify();
    }
}

#[cfg(target_family = "wasm")]
impl McpOauthModel {
    pub fn connect(&mut self, _entry: &McpServerEntry, _ctx: &mut ModelContext<Self>) {}
    pub fn disconnect(&mut self, _server_id: &str, _ctx: &mut ModelContext<Self>) {}
    pub fn forget(&mut self, _server_id: &str, _ctx: &mut ModelContext<Self>) {}
    pub fn restore(&mut self, _entries: &[McpServerEntry], _ctx: &mut ModelContext<Self>) {}
}

impl Entity for McpOauthModel {
    type Event = ();
}

impl SingletonEntity for McpOauthModel {}

/// twarp 24b: route a `twarp://mcp/oauth2callback` URL to the pending flow.
/// Called from [`crate::uri::handle_incoming_uri`].
pub fn handle_oauth_callback(url: &url::Url, ctx: &mut AppContext) {
    match Callback::parse(url) {
        Some(Callback::Granted {
            server_id,
            code,
            state,
        }) => McpOauthModel::handle(ctx).update(ctx, |model, ctx| {
            model.complete_authorization(&server_id, code, state, ctx);
        }),
        Some(Callback::Denied { server_id, reason }) => {
            McpOauthModel::handle(ctx).update(ctx, |model, ctx| {
                model.fail_pending(&server_id, reason, ctx);
            })
        }
        None => log::warn!("Ignoring malformed MCP OAuth callback: {url}"),
    }
}

/// A parsed `twarp://mcp/oauth2callback` redirect.
#[derive(Debug, PartialEq, Eq)]
enum Callback {
    Granted {
        server_id: String,
        code: String,
        state: String,
    },
    Denied {
        server_id: String,
        reason: String,
    },
}

impl Callback {
    fn parse(url: &url::Url) -> Option<Self> {
        let params: HashMap<_, _> = url.query_pairs().into_owned().collect();
        // Without a server id there is nothing to route to; the flow times out
        // instead of guessing.
        let server_id = params.get("server_id").cloned()?;

        // A refused consent comes back as `?error=access_denied`, no code.
        if let Some(error) = params.get("error") {
            let reason = params
                .get("error_description")
                .filter(|description| !description.is_empty())
                .unwrap_or(error)
                .clone();
            return Some(Callback::Denied { server_id, reason });
        }

        let code = params.get("code").cloned()?;
        let state = params.get("state").cloned()?;
        Some(Callback::Granted {
            server_id,
            code,
            state,
        })
    }
}

#[cfg(target_family = "wasm")]
impl McpOauthModel {
    fn fail_pending(&mut self, _server_id: &str, _message: String, _ctx: &mut ModelContext<Self>) {}
    fn complete_authorization(
        &mut self,
        _server_id: &str,
        _code: String,
        _state: String,
        _ctx: &mut ModelContext<Self>,
    ) {
    }
}

#[cfg(not(target_family = "wasm"))]
mod imp {
    //! The parts that need tokio, reqwest and rmcp — i.e. everything except
    //! the status bookkeeping above.

    use std::collections::BTreeMap;
    use std::sync::Arc;

    use async_trait::async_trait;
    use oauth2::TokenResponse;
    use rmcp::transport::auth::{
        AuthError, AuthorizationManager, CredentialStore, OAuthState, StoredCredentials,
    };
    use rmcp::transport::{StreamableHttpClientTransport, streamable_http_client::StreamableHttpClientTransportConfig};
    use rmcp::{ServiceExt, model::{ClientCapabilities, ClientInfo, Implementation}};
    use tokio::sync::Mutex as TokioMutex;
    use tokio::sync::RwLock as TokioRwLock;
    use twarp_core::channel::ChannelState;
    use twarpui::{ModelContext, ModelSpawner, SingletonEntity};

    use crate::mcp_registry::{McpAuth, McpRegistryModel, McpServerEntry, McpTransport};

    use super::{
        CONSENT_TIMEOUT, McpAuthStatus, McpOauthModel, REFRESH_INTERVAL, keychain_key,
        redirect_uri,
    };

    /// Scopes requested at authorization time. Empty means "whatever the
    /// provider grants by default", which is what hosted MCP servers expect —
    /// they scope by the connected account, not by OAuth scope.
    const SCOPES: &[&str] = &[];

    /// Client name sent during dynamic client registration; shows up on the
    /// provider's consent screen.
    const CLIENT_NAME: &str = "twarp";

    /// Long enough for a cold hosted server, short enough that a wedged
    /// endpoint doesn't hold the row in Connecting forever.
    const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

    pub(super) struct Runtime {
        runtime: tokio::runtime::Runtime,
        spawner: ModelSpawner<McpOauthModel>,
        /// Flows waiting on the browser, keyed by server id.
        pending: std::sync::Mutex<BTreeMap<String, PendingFlow>>,
    }

    struct PendingFlow {
        /// The `state` query parameter the provider must echo back.
        csrf: String,
        /// Holds the PKCE verifier for this flow, so it must survive from
        /// `start_authorization` until the callback arrives.
        session: Arc<TokioMutex<OAuthState>>,
        store: SharedStore,
    }

    impl Runtime {
        pub(super) fn start(ctx: &mut ModelContext<McpOauthModel>) -> Result<Self, String> {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .thread_name("mcp-oauth")
                .build()
                .map_err(|err| err.to_string())?;
            Ok(Self {
                runtime,
                spawner: ctx.spawner(),
                pending: std::sync::Mutex::new(BTreeMap::new()),
            })
        }
    }

    /// A [`CredentialStore`] the main thread can read and seed, so the
    /// keychain (which is only reachable through the app context) stays on
    /// the main thread while rmcp's async machinery talks to this.
    #[derive(Clone, Default)]
    struct SharedStore(Arc<TokioRwLock<Option<StoredCredentials>>>);

    impl SharedStore {
        fn seeded(credentials: Option<StoredCredentials>) -> Self {
            Self(Arc::new(TokioRwLock::new(credentials)))
        }

        async fn snapshot(&self) -> Option<StoredCredentials> {
            self.0.read().await.clone()
        }
    }

    #[async_trait]
    impl CredentialStore for SharedStore {
        async fn load(&self) -> Result<Option<StoredCredentials>, AuthError> {
            Ok(self.0.read().await.clone())
        }

        async fn save(&self, credentials: StoredCredentials) -> Result<(), AuthError> {
            *self.0.write().await = Some(credentials);
            Ok(())
        }

        async fn clear(&self) -> Result<(), AuthError> {
            *self.0.write().await = None;
            Ok(())
        }
    }

    /// What a probe learned about a remote server.
    enum Probe {
        /// Handshake succeeded; the server advertised this many tools.
        Ok { tools: usize },
        /// 401 and the OAuth metadata is discoverable — we can run consent.
        NeedsOauth,
        /// 401 but no OAuth metadata: the user must supply a header.
        NeedsStaticAuth,
        Failed(String),
    }

    /// One MCP handshake against `url`, with `headers` applied verbatim.
    async fn probe(url: String, headers: BTreeMap<String, String>) -> Probe {
        let client = match http_client(&headers) {
            Ok(client) => client,
            Err(err) => return Probe::Failed(err),
        };

        // Preflight so a 401 is classified from the HTTP status rather than
        // guessed at from an opaque rmcp handshake error.
        match preflight(&client, &url).await {
            Ok(true) => {}
            Ok(false) => {
                return if discoverable_oauth(&url).await {
                    Probe::NeedsOauth
                } else {
                    Probe::NeedsStaticAuth
                };
            }
            Err(err) => return Probe::Failed(err),
        }

        let config = StreamableHttpClientTransportConfig::with_uri(url.clone());
        let transport = StreamableHttpClientTransport::with_client(client, config);
        let client_info = ClientInfo {
            protocol_version: Default::default(),
            capabilities: ClientCapabilities::default(),
            client_info: Implementation {
                name: CLIENT_NAME.to_owned(),
                title: None,
                version: env!("CARGO_PKG_VERSION").to_owned(),
                website_url: None,
                icons: None,
            },
        };

        let service = match tokio::time::timeout(PROBE_TIMEOUT, client_info.serve(transport)).await
        {
            Ok(Ok(service)) => service,
            Ok(Err(err)) => return Probe::Failed(format!("MCP handshake failed: {err}")),
            Err(_) => return Probe::Failed("Timed out connecting to the server.".to_owned()),
        };
        let tools = match service.list_tools(Default::default()).await {
            Ok(result) => result.tools.len(),
            Err(err) => {
                let _ = service.cancel().await;
                return Probe::Failed(format!("Could not list tools: {err}"));
            }
        };
        let _ = service.cancel().await;
        Probe::Ok { tools }
    }

    fn http_client(headers: &BTreeMap<String, String>) -> Result<reqwest::Client, String> {
        let mut header_map = reqwest::header::HeaderMap::new();
        for (name, value) in headers {
            let name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| format!("'{name}' is not a valid header name"))?;
            let value = reqwest::header::HeaderValue::from_str(value)
                .map_err(|_| format!("'{name}' has a value that can't be sent as a header"))?;
            header_map.insert(name, value);
        }
        reqwest::Client::builder()
            .default_headers(header_map)
            .timeout(PROBE_TIMEOUT)
            .build()
            .map_err(|err| err.to_string())
    }

    /// POSTs a minimal `initialize` and reports whether the server let us in.
    /// `Ok(false)` means "authorization required"; transport failures are
    /// `Err`, so an offline machine reads as an error rather than as a
    /// credential problem (P15).
    async fn preflight(client: &reqwest::Client, url: &str) -> Result<bool, String> {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": CLIENT_NAME, "version": env!("CARGO_PKG_VERSION") },
            },
        });
        let response = client
            .post(url)
            .header(
                reqwest::header::ACCEPT,
                "application/json, text/event-stream",
            )
            .json(&body)
            .send()
            .await
            .map_err(|err| friendly_transport_error(&err))?;
        Ok(!matches!(
            response.status(),
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
        ))
    }

    fn friendly_transport_error(err: &reqwest::Error) -> String {
        if err.is_timeout() {
            "Timed out connecting to the server.".to_owned()
        } else if err.is_connect() {
            "Could not reach the server.".to_owned()
        } else {
            err.to_string()
        }
    }

    /// Whether the server publishes OAuth metadata (RFC 9728 protected
    /// resource metadata, or RFC 8414 authorization-server metadata).
    async fn discoverable_oauth(url: &str) -> bool {
        match AuthorizationManager::new(url).await {
            Ok(manager) => manager.discover_metadata().await.is_ok(),
            Err(_) => false,
        }
    }

    /// Warns when the user is connecting to an issuer we ship pre-registered
    /// credentials for. Actually *using* them needs an rmcp seam we don't
    /// have: `AuthorizationManager::configure_client` requires discovered
    /// metadata, and neither the `metadata` field nor a setter for it is
    /// public, so an out-of-crate caller can only reach the dynamic-client-
    /// registration path inside `OAuthState::start_authorization`. twarp
    /// ships no static providers today, so this only ever logs; see the
    /// follow-up in `roadmap/24-plugin-auth/TECH.md`.
    fn warn_if_static_provider(url: &str) {
        let Some(issuer) = url::Url::parse(url)
            .ok()
            .and_then(|parsed| parsed.host_str().map(|host| format!("https://{host}")))
        else {
            return;
        };
        if ChannelState::mcp_oauth_provider_by_issuer(&issuer).is_some() {
            log::warn!(
                "{issuer} has statically-bundled MCP OAuth credentials, but only dynamic \
                 client registration is reachable; falling back to DCR"
            );
        }
    }

    impl McpOauthModel {
        /// Clone the pieces a background flow needs, releasing the borrow on
        /// `self.imp` so the caller can keep mutating status.
        fn parts(&self) -> Option<(ModelSpawner<Self>, tokio::runtime::Handle)> {
            let imp = self.imp.as_ref()?;
            Some((imp.spawner.clone(), imp.runtime.handle().clone()))
        }

        /// Drop any flow waiting on the browser for `server_id`.
        fn remove_pending(&self, server_id: &str) {
            if let Some(imp) = &self.imp {
                if let Ok(mut pending) = imp.pending.lock() {
                    pending.remove(server_id);
                }
            }
        }

        /// Probe `entry`, and start browser consent if it turns out to speak
        /// OAuth. Safe to call repeatedly: an in-flight flow for the same
        /// server is superseded (P13).
        pub fn connect(&mut self, entry: &McpServerEntry, ctx: &mut ModelContext<Self>) {
            if entry.transport != McpTransport::Http {
                return;
            }
            let Some(url) = entry.url.clone().filter(|url| !url.is_empty()) else {
                return;
            };
            let Some((spawner, runtime)) = self.parts() else {
                self.set_status(
                    &entry.id,
                    McpAuthStatus::Error("The MCP OAuth runtime failed to start.".to_owned()),
                    ctx,
                );
                return;
            };

            let server_id = entry.id.clone();
            let generation = self.next_generation(&server_id);
            self.remove_pending(&server_id);
            self.set_status(&server_id, McpAuthStatus::Connecting, ctx);

            let headers = entry.resolved_headers(self.tokens.get(&server_id).map(String::as_str));
            let wants_oauth = entry.auth == McpAuth::Oauth;
            runtime.spawn(async move {
                match probe(url.clone(), headers).await {
                    Probe::Ok { tools } => {
                        let _ = spawner
                            .spawn(move |me, ctx| {
                                if me.is_current(&server_id, generation) {
                                    me.set_status(
                                        &server_id,
                                        McpAuthStatus::Connected { tools },
                                        ctx,
                                    );
                                }
                            })
                            .await;
                    }
                    Probe::NeedsOauth => {
                        start_authorization(spawner, server_id, generation, url).await;
                    }
                    Probe::NeedsStaticAuth => {
                        // An OAuth server whose cached token just expired
                        // beyond refresh looks identical to a static-auth
                        // server on the wire, so trust what we recorded.
                        let status = if wants_oauth {
                            McpAuthStatus::NotConnected
                        } else {
                            McpAuthStatus::NeedsStaticAuth
                        };
                        let _ = spawner
                            .spawn(move |me, ctx| {
                                if me.is_current(&server_id, generation) {
                                    me.tokens.remove(&server_id);
                                    me.set_status(&server_id, status, ctx);
                                }
                            })
                            .await;
                    }
                    Probe::Failed(message) => {
                        let _ = spawner
                            .spawn(move |me, ctx| {
                                if me.is_current(&server_id, generation) {
                                    me.set_status(&server_id, McpAuthStatus::Error(message), ctx);
                                }
                            })
                            .await;
                    }
                }
            });
        }

        /// Drop this server's tokens (keychain included) and return it to
        /// Needs authorization. The grant itself stays live on the provider's
        /// side; this is a local revoke (P10).
        pub fn disconnect(&mut self, server_id: &str, ctx: &mut ModelContext<Self>) {
            self.next_generation(server_id);
            self.tokens.remove(server_id);
            self.remove_pending(server_id);
            clear_keychain(server_id, ctx);
            self.set_status(server_id, McpAuthStatus::NotConnected, ctx);
        }

        /// Forget a deleted server entirely (P16).
        pub fn forget(&mut self, server_id: &str, ctx: &mut ModelContext<Self>) {
            self.disconnect(server_id, ctx);
            self.status.remove(server_id);
            self.generation.remove(server_id);
        }

        /// Seed the token cache from the keychain at startup and start the
        /// refresh loop. Servers that still hold a usable token come back as
        /// Connected without any user action.
        pub fn restore(&mut self, entries: &[McpServerEntry], ctx: &mut ModelContext<Self>) {
            let oauth_servers: Vec<(String, String)> = entries
                .iter()
                .filter(|entry| entry.auth == McpAuth::Oauth)
                .filter_map(|entry| Some((entry.id.clone(), entry.url.clone()?)))
                .collect();
            if oauth_servers.is_empty() {
                return;
            }
            for (server_id, _) in &oauth_servers {
                self.status
                    .insert(server_id.to_owned(), McpAuthStatus::Connecting);
            }
            ctx.notify();
            self.refresh_tokens(oauth_servers, ctx);
        }

        /// Refresh every listed server's access token off the main thread,
        /// then re-arm. Refresh failures flip the row to NotConnected rather
        /// than Error: an expired grant is a "press Connect again", not a
        /// malfunction (P7).
        fn refresh_tokens(&mut self, servers: Vec<(String, String)>, ctx: &mut ModelContext<Self>) {
            let Some((spawner, runtime)) = self.parts() else {
                return;
            };
            // Skip only servers waiting on the browser: bumping one of those
            // generations would invalidate the pending consent flow, so the
            // callback would arrive stale and be dropped. `Connecting` is NOT
            // skipped — `restore` sets it on every server as its seeding pass,
            // and a superseded probe just re-probes.
            let servers: Vec<(String, String)> = servers
                .into_iter()
                .filter(|(id, _)| {
                    !matches!(
                        self.status.get(id),
                        Some(McpAuthStatus::WaitingForBrowser)
                    )
                })
                .collect();
            if servers.is_empty() {
                self.rearm_refresh(ctx);
                return;
            }
            let generations: Vec<u64> = servers
                .iter()
                .map(|(id, _)| id.clone())
                .map(|id| self.next_generation(&id))
                .collect();
            let _ = ctx;

            runtime.spawn(async move {
                for ((server_id, url), generation) in servers.into_iter().zip(generations) {
                    let key_id = server_id.clone();
                    let stored = spawner
                        .spawn(move |_, ctx| read_keychain(&key_id, ctx))
                        .await
                        .ok()
                        .flatten();
                    let Some(stored) = stored else {
                        let _ = spawner
                            .spawn(move |me, ctx| {
                                if me.is_current(&server_id, generation) {
                                    me.set_status(&server_id, McpAuthStatus::NotConnected, ctx);
                                }
                            })
                            .await;
                        continue;
                    };

                    let store = SharedStore::seeded(Some(stored));
                    match access_token(&url, store.clone()).await {
                        Ok(token) => {
                            let snapshot = store.snapshot().await;
                            let _ = spawner
                                .spawn(move |me, ctx| {
                                    if !me.is_current(&server_id, generation) {
                                        return;
                                    }
                                    if let Some(snapshot) = snapshot {
                                        write_keychain(&server_id, &snapshot, ctx);
                                    }
                                    me.tokens.insert(server_id.clone(), token);
                                    // Tool count is unknown until a probe; a
                                    // refreshed token is enough to call the
                                    // row connected with zero known tools
                                    // only if we never learned better.
                                    let status = me
                                        .status
                                        .get(&server_id)
                                        .filter(|status| status.is_connected())
                                        .cloned()
                                        .unwrap_or(McpAuthStatus::Connected { tools: 0 });
                                    me.set_status(&server_id, status, ctx);
                                })
                                .await;
                        }
                        Err(err) => {
                            log::warn!("MCP OAuth token refresh failed for {server_id}: {err}");
                            let _ = spawner
                                .spawn(move |me, ctx| {
                                    if me.is_current(&server_id, generation) {
                                        me.tokens.remove(&server_id);
                                        me.set_status(&server_id, McpAuthStatus::NotConnected, ctx);
                                    }
                                })
                                .await;
                        }
                    }
                }

                // One re-arm path for the whole loop (see `rearm_refresh`),
                // so there is a single timer rather than one here and one
                // there.
                let _ = spawner.spawn(|me, ctx| me.rearm_refresh(ctx)).await;
            });
        }

        /// Schedule the next refresh pass. The only place the loop's interval
        /// is applied — both the end of a pass and an all-skipped pass come
        /// through here, so the loop can't die out or double up.
        fn rearm_refresh(&mut self, ctx: &mut ModelContext<Self>) {
            ctx.spawn(
                async move {
                    twarpui::r#async::Timer::after(REFRESH_INTERVAL).await;
                },
                |me: &mut Self, _, ctx| {
                    let servers = oauth_servers(ctx);
                    me.refresh_tokens(servers, ctx);
                },
            );
        }

        /// Exchange the authorization code for tokens and finish the flow.
        pub(super) fn complete_authorization(
            &mut self,
            server_id: &str,
            code: String,
            state: String,
            ctx: &mut ModelContext<Self>,
        ) {
            let Some((spawner, runtime)) = self.parts() else {
                return;
            };
            let Some(flow) = self
                .imp
                .as_ref()
                .and_then(|imp| imp.pending.lock().ok()?.remove(server_id))
            else {
                log::warn!("MCP OAuth callback for {server_id} has no pending flow");
                return;
            };
            if flow.csrf != state {
                self.set_status(
                    server_id,
                    McpAuthStatus::Error("Authorization response did not match.".to_owned()),
                    ctx,
                );
                return;
            }

            let server_id = server_id.to_owned();
            let generation = self.next_generation(&server_id);
            self.set_status(&server_id, McpAuthStatus::Connecting, ctx);
            let csrf = flow.csrf.clone();

            runtime.spawn(async move {
                let mut session = flow.session.lock().await;
                // Exchanges the code using the PKCE verifier this session has
                // been holding, and rejects a mismatched `state` itself.
                if let Err(err) = session.handle_callback(&code, &csrf).await {
                    let message = format!("Could not complete authorization: {err}");
                    let _ = spawner
                        .spawn(move |me, ctx| {
                            if me.is_current(&server_id, generation) {
                                me.set_status(&server_id, McpAuthStatus::Error(message), ctx);
                            }
                        })
                        .await;
                    return;
                }
                drop(session);

                // The exchange writes through to our store, so that — not
                // `OAuthState`, which refuses token reads once authorized —
                // is where the token comes from.
                let snapshot = flow.store.snapshot().await;
                let Some(token) = snapshot.as_ref().and_then(access_token_of) else {
                    let _ = spawner
                        .spawn(move |me, ctx| {
                            if me.is_current(&server_id, generation) {
                                me.set_status(
                                    &server_id,
                                    McpAuthStatus::Error(
                                        "The provider returned no access token.".to_owned(),
                                    ),
                                    ctx,
                                );
                            }
                        })
                        .await;
                    return;
                };

                // Persist first, then re-probe so the chip can show the tool
                // count the user actually gained.
                let persisted_id = server_id.clone();
                let persisted_token = token.clone();
                let url = spawner
                    .spawn(move |me, ctx| {
                        if let Some(snapshot) = snapshot {
                            write_keychain(&persisted_id, &snapshot, ctx);
                        }
                        me.tokens.insert(persisted_id.clone(), persisted_token);
                        mark_entry_oauth(&persisted_id, ctx)
                    })
                    .await
                    .ok()
                    .flatten();

                let Some(url) = url else {
                    let _ = spawner
                        .spawn(move |me, ctx| {
                            me.set_status(&server_id, McpAuthStatus::Connected { tools: 0 }, ctx);
                        })
                        .await;
                    return;
                };

                let mut headers = BTreeMap::new();
                headers.insert("Authorization".to_owned(), format!("Bearer {token}"));
                let tools = match probe(url, headers).await {
                    Probe::Ok { tools } => tools,
                    _ => 0,
                };
                let _ = spawner
                    .spawn(move |me, ctx| {
                        if me.is_current(&server_id, generation) {
                            me.set_status(&server_id, McpAuthStatus::Connected { tools }, ctx);
                        }
                    })
                    .await;
            });
        }

        /// Abandon a pending flow (consent denied, Cancel, or timeout).
        pub(super) fn fail_pending(
            &mut self,
            server_id: &str,
            message: String,
            ctx: &mut ModelContext<Self>,
        ) {
            self.remove_pending(server_id);
            self.next_generation(server_id);
            self.set_status(server_id, McpAuthStatus::Error(message), ctx);
        }

        /// User pressed Cancel while the browser was open.
        pub fn cancel(&mut self, server_id: &str, ctx: &mut ModelContext<Self>) {
            self.remove_pending(server_id);
            self.next_generation(server_id);
            self.set_status(server_id, McpAuthStatus::NotConnected, ctx);
        }
    }

    /// Discovery + DCR + PKCE, then hand the consent URL to the main thread
    /// to open in the browser.
    async fn start_authorization(
        spawner: ModelSpawner<McpOauthModel>,
        server_id: String,
        generation: u64,
        url: String,
    ) {
        let stored_id = server_id.clone();
        let stored = spawner
            .spawn(move |_, ctx| read_keychain(&stored_id, ctx))
            .await
            .ok()
            .flatten();
        let store = SharedStore::seeded(stored);

        // A stored grant that still refreshes means no consent screen at all.
        if store.snapshot().await.is_some() {
            if let Ok(token) = access_token(&url, store.clone()).await {
                let snapshot = store.snapshot().await;
                let probe_url = url.clone();
                let mut headers = BTreeMap::new();
                headers.insert("Authorization".to_owned(), format!("Bearer {token}"));
                let tools = match probe(probe_url, headers).await {
                    Probe::Ok { tools } => tools,
                    _ => 0,
                };
                let _ = spawner
                    .spawn(move |me, ctx| {
                        if !me.is_current(&server_id, generation) {
                            return;
                        }
                        if let Some(snapshot) = snapshot {
                            write_keychain(&server_id, &snapshot, ctx);
                        }
                        me.tokens.insert(server_id.clone(), token);
                        me.set_status(&server_id, McpAuthStatus::Connected { tools }, ctx);
                    })
                    .await;
                return;
            }
        }

        let session = match build_session(&url, store.clone(), &server_id).await {
            Ok(session) => session,
            Err(err) => {
                let message = format!("Could not start authorization: {err}");
                let _ = spawner
                    .spawn(move |me, ctx| {
                        if me.is_current(&server_id, generation) {
                            me.set_status(&server_id, McpAuthStatus::Error(message), ctx);
                        }
                    })
                    .await;
                return;
            }
        };

        let auth_url = match session.get_authorization_url().await {
            Ok(auth_url) => auth_url,
            Err(err) => {
                let message = format!("Could not start authorization: {err}");
                let _ = spawner
                    .spawn(move |me, ctx| {
                        if me.is_current(&server_id, generation) {
                            me.set_status(&server_id, McpAuthStatus::Error(message), ctx);
                        }
                    })
                    .await;
                return;
            }
        };
        let Some(csrf) = csrf_from(&auth_url) else {
            let _ = spawner
                .spawn(move |me, ctx| {
                    if me.is_current(&server_id, generation) {
                        me.set_status(
                            &server_id,
                            McpAuthStatus::Error(
                                "The provider's authorization URL was malformed.".to_owned(),
                            ),
                            ctx,
                        );
                    }
                })
                .await;
            return;
        };

        let flow = PendingFlow {
            csrf,
            session: Arc::new(TokioMutex::new(session)),
            store,
        };
        let _ = spawner
            .spawn(move |me, ctx| {
                if !me.is_current(&server_id, generation) {
                    return;
                }
                if let Some(imp) = &me.imp {
                    if let Ok(mut pending) = imp.pending.lock() {
                        pending.insert(server_id.clone(), flow);
                    }
                }
                me.set_status(&server_id, McpAuthStatus::WaitingForBrowser, ctx);
                ctx.open_url(&auth_url);
                me.arm_consent_timeout(server_id, generation, ctx);
            })
            .await;
    }

    /// Discovery + dynamic client registration + PKCE, leaving the flow
    /// parked in [`OAuthState::Session`] with its verifier held for the
    /// callback.
    async fn build_session(
        url: &str,
        store: SharedStore,
        server_id: &str,
    ) -> Result<OAuthState, AuthError> {
        let mut state = OAuthState::new(url, None).await?;
        // Seeding the store before authorization means the exchanged tokens
        // land somewhere the main thread can read and mirror to the keychain.
        if let OAuthState::Unauthorized(manager) = &mut state {
            manager.set_credential_store(store);
        }
        warn_if_static_provider(url);
        state
            .start_authorization(SCOPES, &redirect_uri(server_id), Some(CLIENT_NAME))
            .await?;
        Ok(state)
    }

    /// The access token inside a set of stored credentials, if any.
    fn access_token_of(credentials: &StoredCredentials) -> Option<String> {
        credentials
            .token_response
            .as_ref()
            .map(|response| response.access_token().secret().clone())
    }

    /// Load-or-refresh the access token for a server whose credentials are
    /// already in `store`.
    async fn access_token(url: &str, store: SharedStore) -> Result<String, AuthError> {
        let mut manager = AuthorizationManager::new(url).await?;
        manager.set_credential_store(store);
        if !manager.initialize_from_store().await? {
            return Err(AuthError::InternalError(
                "no stored credentials".to_owned(),
            ));
        }
        manager.get_access_token().await
    }

    /// The `state` parameter the provider will echo back to us.
    fn csrf_from(auth_url: &str) -> Option<String> {
        url::Url::parse(auth_url).ok().and_then(|parsed| {
            parsed
                .query_pairs()
                .find(|(key, _)| key == "state")
                .map(|(_, value)| value.into_owned())
        })
    }

    impl McpOauthModel {
        /// Return the row to a retriable state if consent never comes back
        /// (P6). Runs on the main thread; a superseded generation no-ops.
        fn arm_consent_timeout(
            &mut self,
            server_id: String,
            generation: u64,
            ctx: &mut ModelContext<Self>,
        ) {
            ctx.spawn(
                async move {
                    twarpui::r#async::Timer::after(CONSENT_TIMEOUT).await;
                },
                move |me: &mut Self, _, ctx| {
                    if me.is_current(&server_id, generation) {
                        me.fail_pending(
                            &server_id,
                            "Timed out waiting for the browser.".to_owned(),
                            ctx,
                        );
                    }
                },
            );
        }
    }

    /// The OAuth servers currently in the registry, for the refresh loop.
    fn oauth_servers(ctx: &mut ModelContext<McpOauthModel>) -> Vec<(String, String)> {
        crate::mcp_registry::McpRegistryModel::as_ref(ctx)
            .servers()
            .iter()
            .filter(|entry| entry.auth == crate::mcp_registry::McpAuth::Oauth)
            .filter_map(|entry| Some((entry.id.clone(), entry.url.clone()?)))
            .collect()
    }

    /// Record that this server authenticates by OAuth now, and hand back its
    /// URL for the confirming probe.
    fn mark_entry_oauth(server_id: &str, ctx: &mut ModelContext<McpOauthModel>) -> Option<String> {
        let registry = crate::mcp_registry::McpRegistryModel::handle(ctx);
        registry.update(ctx, |registry, ctx| {
            let mut entry = registry.get(server_id)?.clone();
            let url = entry.url.clone();
            if entry.auth != crate::mcp_registry::McpAuth::Oauth {
                entry.auth = crate::mcp_registry::McpAuth::Oauth;
                registry.upsert(entry, ctx);
            }
            url
        })
    }

    fn read_keychain(
        server_id: &str,
        ctx: &mut ModelContext<McpOauthModel>,
    ) -> Option<StoredCredentials> {
        use twarpui_extras::secure_storage::AppContextExt;
        let raw = ctx.secure_storage().read_value(&keychain_key(server_id));
        match raw {
            Ok(raw) => serde_json::from_str(&raw)
                .inspect_err(|err| {
                    log::warn!("Stored MCP OAuth credentials for {server_id} are unreadable: {err}")
                })
                .ok(),
            Err(twarpui_extras::secure_storage::Error::NotFound) => None,
            Err(err) => {
                log::warn!("Could not read MCP OAuth credentials for {server_id}: {err}");
                None
            }
        }
    }

    fn write_keychain(
        server_id: &str,
        credentials: &StoredCredentials,
        ctx: &mut ModelContext<McpOauthModel>,
    ) {
        use twarpui_extras::secure_storage::AppContextExt;
        let Ok(serialized) = serde_json::to_string(credentials) else {
            return;
        };
        if let Err(err) = ctx
            .secure_storage()
            .write_value(&keychain_key(server_id), &serialized)
        {
            log::error!("Could not store MCP OAuth credentials for {server_id}: {err}");
        }
    }

    pub(super) fn clear_keychain(server_id: &str, ctx: &mut ModelContext<McpOauthModel>) {
        use twarpui_extras::secure_storage::AppContextExt;
        match ctx.secure_storage().remove_value(&keychain_key(server_id)) {
            Ok(()) | Err(twarpui_extras::secure_storage::Error::NotFound) => {}
            Err(err) => {
                log::warn!("Could not remove MCP OAuth credentials for {server_id}: {err}")
            }
        }
    }
}

#[cfg(not(target_family = "wasm"))]
use imp::clear_keychain;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keychain_key_is_namespaced_per_server() {
        assert_eq!(keychain_key("abc-123"), "mcp.oauth.abc-123");
    }

    #[test]
    fn redirect_uri_carries_the_server_id() {
        assert_eq!(
            redirect_uri("abc-123"),
            "twarp://mcp/oauth2callback?server_id=abc-123"
        );
    }

    #[test]
    fn status_labels_read_naturally() {
        assert_eq!(McpAuthStatus::Connected { tools: 1 }.label(), "Connected · 1 tool");
        assert_eq!(
            McpAuthStatus::Connected { tools: 12 }.label(),
            "Connected · 12 tools"
        );
        assert_eq!(McpAuthStatus::Local.label(), "Local");
    }

    fn callback(url: &str) -> Option<Callback> {
        Callback::parse(&url::Url::parse(url).expect("test url"))
    }

    #[test]
    fn a_granted_callback_carries_server_code_and_state() {
        assert_eq!(
            callback("twarp://mcp/oauth2callback?server_id=s1&code=c1&state=st1"),
            Some(Callback::Granted {
                server_id: "s1".to_owned(),
                code: "c1".to_owned(),
                state: "st1".to_owned(),
            })
        );
    }

    #[test]
    fn a_denied_callback_reports_the_providers_reason() {
        // Consent refused: description when there is one, else the raw code.
        assert_eq!(
            callback(
                "twarp://mcp/oauth2callback?server_id=s1&error=access_denied                 &error_description=User%20said%20no"
            ),
            Some(Callback::Denied {
                server_id: "s1".to_owned(),
                reason: "User said no".to_owned(),
            })
        );
        assert_eq!(
            callback("twarp://mcp/oauth2callback?server_id=s1&error=access_denied"),
            Some(Callback::Denied {
                server_id: "s1".to_owned(),
                reason: "access_denied".to_owned(),
            })
        );
    }

    #[test]
    fn incomplete_callbacks_are_ignored() {
        // Each of these would otherwise route a flow nowhere, or complete one
        // without the material to exchange.
        assert_eq!(callback("twarp://mcp/oauth2callback?code=c1&state=st1"), None);
        assert_eq!(callback("twarp://mcp/oauth2callback?server_id=s1&code=c1"), None);
        assert_eq!(
            callback("twarp://mcp/oauth2callback?server_id=s1&state=st1"),
            None
        );
        assert_eq!(callback("twarp://mcp/oauth2callback"), None);
    }

    #[test]
    fn only_pending_states_are_busy() {
        assert!(McpAuthStatus::Connecting.is_busy());
        assert!(McpAuthStatus::WaitingForBrowser.is_busy());
        assert!(!McpAuthStatus::NotConnected.is_busy());
        assert!(!McpAuthStatus::Connected { tools: 3 }.is_busy());
    }
}
