//! twarp 20b: the user-managed shared MCP-server registry, kept in memory and
//! mirrored to the `mcp_servers` SQLite table.
//!
//! The registry is edited on the Automation > MCPs page and injected into
//! provider sessions at spawn time:
//! - Claude: merged into the inline `--mcp-config` JSON (built-ins like the
//!   browser bridge keep priority on a name collision).
//! - Codex: passed as `mcp_servers` config overrides in the app-server
//!   `thread/start` / `thread/resume` request's `config` object.
//!
//! Changes take effect for NEW sessions only; running sessions keep the
//! config they were spawned with.

use std::collections::BTreeMap;

use twarpui::{Entity, ModelContext, SingletonEntity};

use crate::persistence::{ModelEvent, PersistedMcpServer};
use crate::GlobalResourceHandlesProvider;

/// How a registry server is reached.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum McpTransport {
    /// A local process spoken to over stdio (`command` + `args` + `env`).
    #[default]
    Stdio,
    /// A remote server spoken to over (streamable) HTTP (`url`).
    Http,
}

impl McpTransport {
    pub fn as_str(self) -> &'static str {
        match self {
            McpTransport::Stdio => "stdio",
            McpTransport::Http => "http",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "stdio" => Some(McpTransport::Stdio),
            "http" => Some(McpTransport::Http),
            _ => None,
        }
    }

    /// Label shown in the transport dropdown.
    pub fn label(self) -> &'static str {
        match self {
            McpTransport::Stdio => "Command (stdio)",
            McpTransport::Http => "URL (HTTP)",
        }
    }
}

/// twarp 24a: how a remote (HTTP) server authenticates. Stdio servers are
/// always [`McpAuth::None`] — a local process inherits `env`, not headers.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum McpAuth {
    /// No auth, or the credential is baked into the URL (the pre-24a shape
    /// every existing entry migrates to).
    #[default]
    None,
    /// Static credentials the user pasted into the Headers field.
    Headers,
    /// twarp 24b: browser-consent OAuth; the token lives in the keychain
    /// under `mcp.oauth.<id>` and is injected at session spawn.
    Oauth,
}

impl McpAuth {
    pub fn as_str(self) -> &'static str {
        match self {
            McpAuth::None => "none",
            McpAuth::Headers => "headers",
            McpAuth::Oauth => "oauth",
        }
    }

    /// Pre-24a rows have a NULL `auth` column and are treated as [`Self::None`],
    /// which is what keeps URL-with-embedded-key servers working untouched.
    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "none" => Some(McpAuth::None),
            "headers" => Some(McpAuth::Headers),
            "oauth" => Some(McpAuth::Oauth),
            _ => None,
        }
    }
}

/// One user-managed MCP-server registry entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct McpServerEntry {
    /// UUID string, stable across edits.
    pub id: String,
    pub name: String,
    pub transport: McpTransport,
    /// Set when `transport == Stdio`.
    pub command: Option<String>,
    pub args: Vec<String>,
    /// Set when `transport == Http`.
    pub url: Option<String>,
    /// Sorted for stable JSON output.
    pub env: BTreeMap<String, String>,
    pub enabled_claude: bool,
    pub enabled_codex: bool,
    /// twarp 23a: owning plugin's UUID ([`crate::plugin_registry`]); `None`
    /// until the next load's orphan migration adopts the entry.
    pub plugin_id: Option<String>,
    /// twarp 24a: extra HTTP headers sent to a remote server, sorted for
    /// stable JSON output. Ignored for stdio transports.
    pub headers: BTreeMap<String, String>,
    /// twarp 24a: how this server authenticates.
    pub auth: McpAuth,
}

impl McpServerEntry {
    fn from_persisted(row: PersistedMcpServer) -> Option<Self> {
        let transport = McpTransport::from_str(&row.transport)?;
        let args = row
            .args
            .as_deref()
            .and_then(|json| serde_json::from_str::<Vec<String>>(json).ok())
            .unwrap_or_default();
        let env = row
            .env
            .as_deref()
            .and_then(|json| serde_json::from_str::<BTreeMap<String, String>>(json).ok())
            .unwrap_or_default();
        Some(Self {
            id: row.id,
            name: row.name,
            transport,
            command: row.command,
            args,
            url: row.url,
            env,
            enabled_claude: row.enabled_claude,
            enabled_codex: row.enabled_codex,
            plugin_id: row.plugin_id,
            headers: row
                .headers
                .as_deref()
                .and_then(|json| serde_json::from_str::<BTreeMap<String, String>>(json).ok())
                .unwrap_or_default(),
            // A NULL / unrecognized `auth` means a pre-24a row: no auth.
            auth: row
                .auth
                .as_deref()
                .and_then(McpAuth::from_str)
                .unwrap_or_default(),
        })
    }

    fn to_persisted(&self) -> PersistedMcpServer {
        PersistedMcpServer {
            id: self.id.clone(),
            name: self.name.clone(),
            transport: self.transport.as_str().to_owned(),
            command: self.command.clone(),
            args: (!self.args.is_empty())
                .then(|| serde_json::to_string(&self.args).unwrap_or_default()),
            url: self.url.clone(),
            env: (!self.env.is_empty())
                .then(|| serde_json::to_string(&self.env).unwrap_or_default()),
            enabled_claude: self.enabled_claude,
            enabled_codex: self.enabled_codex,
            plugin_id: self.plugin_id.clone(),
            headers: (!self.headers.is_empty())
                .then(|| serde_json::to_string(&self.headers).unwrap_or_default()),
            auth: Some(self.auth.as_str().to_owned()),
        }
    }

    /// Short one-line summary shown under the name on the MCPs page: the
    /// command line for stdio servers, the URL for HTTP servers.
    pub fn summary(&self) -> String {
        match self.transport {
            McpTransport::Stdio => {
                let mut parts = Vec::with_capacity(1 + self.args.len());
                parts.extend(self.command.clone());
                parts.extend(self.args.iter().cloned());
                parts.join(" ")
            }
            McpTransport::Http => self.url.clone().unwrap_or_default(),
        }
    }

    /// twarp 24a: the headers actually sent to a remote server — the entry's
    /// static headers, plus an `Authorization: Bearer` for OAuth servers once
    /// [`crate::mcp_oauth`] has resolved a token. An explicit `Authorization`
    /// header the user typed wins, so a stale cached token can't shadow it.
    pub fn resolved_headers(&self, oauth_token: Option<&str>) -> BTreeMap<String, String> {
        let mut headers = self.headers.clone();
        if self.auth == McpAuth::Oauth {
            if let Some(token) = oauth_token {
                headers
                    .entry(AUTHORIZATION_HEADER.to_owned())
                    .or_insert_with(|| format!("Bearer {token}"));
            }
        }
        headers
    }

    /// The `mcpServers.<name>` value in Claude `--mcp-config` shape.
    fn claude_config_value(&self, oauth_token: Option<&str>) -> serde_json::Value {
        match self.transport {
            McpTransport::Stdio => {
                let mut obj = serde_json::json!({
                    "type": "stdio",
                    "command": self.command.clone().unwrap_or_default(),
                    "args": self.args,
                });
                if !self.env.is_empty() {
                    obj["env"] = serde_json::json!(self.env);
                }
                obj
            }
            McpTransport::Http => {
                let mut obj = serde_json::json!({
                    "type": "http",
                    "url": self.url.clone().unwrap_or_default(),
                });
                let headers = self.resolved_headers(oauth_token);
                if !headers.is_empty() {
                    obj["headers"] = serde_json::json!(headers);
                }
                obj
            }
        }
    }

    /// The `mcp_servers.<name>` value in Codex `config.toml`-override shape
    /// (passed via the app-server `thread/start` `config` object).
    fn codex_config_value(&self, oauth_token: Option<&str>) -> serde_json::Value {
        match self.transport {
            McpTransport::Stdio => {
                let mut obj = serde_json::json!({
                    "command": self.command.clone().unwrap_or_default(),
                    "args": self.args,
                });
                if !self.env.is_empty() {
                    obj["env"] = serde_json::json!(self.env);
                }
                obj
            }
            McpTransport::Http => {
                let mut obj = serde_json::json!({
                    "url": self.url.clone().unwrap_or_default(),
                });
                // twarp 24a: Codex's `mcp_servers` config can only express a
                // bearer token, not arbitrary headers, so a `Authorization:
                // Bearer <token>` header maps onto `bearer_token` and anything
                // else is dropped with a warning (the Plugins page surfaces
                // the same caveat before the user enables Codex).
                let headers = self.resolved_headers(oauth_token);
                let mut unsupported: Vec<&str> = Vec::new();
                for (name, value) in &headers {
                    match bearer_token(name, value) {
                        Some(token) => obj["bearer_token"] = serde_json::json!(token),
                        None => unsupported.push(name.as_str()),
                    }
                }
                if !unsupported.is_empty() {
                    log::warn!(
                        "Codex cannot send custom MCP headers; dropping {} for server '{}'",
                        unsupported.join(", "),
                        self.name
                    );
                }
                obj
            }
        }
    }

    /// twarp 24a: whether this entry has a header Codex cannot express (any
    /// header other than a bearer `Authorization`). Drives the Plugins page
    /// warning; see [`Self::codex_config_value`].
    pub fn has_codex_unsupported_headers(&self) -> bool {
        self.transport == McpTransport::Http
            && self
                .headers
                .iter()
                .any(|(name, value)| bearer_token(name, value).is_none())
    }
}

/// Case-insensitive per RFC 9110; services and users write it both ways.
const AUTHORIZATION_HEADER: &str = "Authorization";

/// twarp 24a: whether this header is a bearer `Authorization` — the only
/// header Codex can carry. Drives the Plugins-page warning.
pub fn is_bearer_authorization(name: &str, value: &str) -> bool {
    bearer_token(name, value).is_some()
}

/// The token in an `Authorization: Bearer <token>` header value, if that's
/// what this header is.
fn bearer_token<'a>(name: &str, value: &'a str) -> Option<&'a str> {
    if !name.eq_ignore_ascii_case(AUTHORIZATION_HEADER) {
        return None;
    }
    let rest = value.strip_prefix("Bearer ").or_else(|| {
        value
            .get(..7)
            .filter(|prefix| prefix.eq_ignore_ascii_case("bearer "))
            .map(|_| &value[7..])
    })?;
    let token = rest.trim();
    (!token.is_empty()).then_some(token)
}

/// Singleton holding the registry; loaded from SQLite at startup, persisted on
/// every mutation by replacing the whole (small) table.
pub struct McpRegistryModel {
    servers: Vec<McpServerEntry>,
}

impl McpRegistryModel {
    pub fn new(persisted: Vec<PersistedMcpServer>) -> Self {
        Self {
            servers: persisted
                .into_iter()
                .filter_map(McpServerEntry::from_persisted)
                .collect(),
        }
    }

    pub fn servers(&self) -> &[McpServerEntry] {
        &self.servers
    }

    pub fn get(&self, id: &str) -> Option<&McpServerEntry> {
        self.servers.iter().find(|s| s.id == id)
    }

    /// Whether `name` is already used by an entry other than `except_id`.
    pub fn name_taken(&self, name: &str, except_id: Option<&str>) -> bool {
        self.servers
            .iter()
            .any(|s| s.name == name && Some(s.id.as_str()) != except_id)
    }

    /// Insert or (matched by id) replace an entry, then persist.
    pub fn upsert(&mut self, entry: McpServerEntry, ctx: &mut ModelContext<Self>) {
        if let Some(existing) = self.servers.iter_mut().find(|s| s.id == entry.id) {
            *existing = entry;
        } else {
            self.servers.push(entry);
        }
        self.servers.sort_by(|a, b| a.name.cmp(&b.name));
        self.persist(ctx);
    }

    pub fn delete(&mut self, id: &str, ctx: &mut ModelContext<Self>) {
        let before = self.servers.len();
        self.servers.retain(|s| s.id != id);
        if self.servers.len() != before {
            self.persist(ctx);
        }
    }

    /// Flip one provider-enable bit on an entry, then persist.
    pub fn toggle_enabled(&mut self, id: &str, claude: bool, ctx: &mut ModelContext<Self>) {
        let Some(entry) = self.servers.iter_mut().find(|s| s.id == id) else {
            return;
        };
        if claude {
            entry.enabled_claude = !entry.enabled_claude;
        } else {
            entry.enabled_codex = !entry.enabled_codex;
        }
        self.persist(ctx);
    }

    /// Registry contribution to the Claude `--mcp-config` inline JSON:
    /// `{"mcpServers": {...}}` over the Claude-enabled entries whose owning
    /// plugin is also Claude-enabled (twarp 23a: effective enablement =
    /// component AND plugin; see `plugin_toggles`), or `None` when there are
    /// none. Built-ins are merged on top by the caller, so they win name
    /// collisions.
    pub fn claude_mcp_config_json(
        &self,
        plugin_toggles: &BTreeMap<String, (bool, bool)>,
        oauth_tokens: &OauthTokens,
    ) -> Option<String> {
        let servers = claude_mcp_servers_map(&self.servers, plugin_toggles, oauth_tokens);
        (!servers.is_empty()).then(|| serde_json::json!({ "mcpServers": servers }).to_string())
    }

    /// Registry contribution to the Codex app-server `thread/start` `config`
    /// overrides: `{"mcp_servers": {...}}` over the Codex-enabled entries
    /// whose owning plugin is also Codex-enabled, or `None` when there are
    /// none.
    pub fn codex_config_overrides(
        &self,
        plugin_toggles: &BTreeMap<String, (bool, bool)>,
        oauth_tokens: &OauthTokens,
    ) -> Option<serde_json::Value> {
        let servers = codex_mcp_servers_map(&self.servers, plugin_toggles, oauth_tokens);
        (!servers.is_empty()).then(|| serde_json::json!({ "mcp_servers": servers }))
    }

    fn persist(&self, ctx: &mut ModelContext<Self>) {
        ctx.notify();
        let handles = GlobalResourceHandlesProvider::as_ref(ctx).get();
        if let Some(sender) = &handles.model_event_sender {
            let event = ModelEvent::ReplaceMcpServers {
                servers: self
                    .servers
                    .iter()
                    .map(McpServerEntry::to_persisted)
                    .collect(),
            };
            if let Err(err) = sender.send(event) {
                log::error!("Failed to persist mcp server registry: {err}");
            }
        }
    }
}

/// twarp 23a: whether a component's owning plugin allows the given provider.
/// Entries with no / a dangling plugin fail open (the next load's migration
/// adopts them into a plugin of their own).
fn plugin_allows(
    plugin_toggles: &BTreeMap<String, (bool, bool)>,
    plugin_id: Option<&str>,
    claude: bool,
) -> bool {
    match plugin_id.and_then(|id| plugin_toggles.get(id)) {
        Some(&(enabled_claude, enabled_codex)) => {
            if claude {
                enabled_claude
            } else {
                enabled_codex
            }
        }
        None => true,
    }
}

/// twarp 24a: resolved OAuth access tokens keyed by server id, supplied by
/// [`crate::mcp_oauth`] at spawn time. A server missing from the map gets no
/// `Authorization` header, so the provider answers 401 and the user
/// reconnects on the Plugins page.
pub type OauthTokens = BTreeMap<String, String>;

fn claude_mcp_servers_map(
    servers: &[McpServerEntry],
    plugin_toggles: &BTreeMap<String, (bool, bool)>,
    oauth_tokens: &OauthTokens,
) -> serde_json::Map<String, serde_json::Value> {
    servers
        .iter()
        .filter(|s| {
            s.enabled_claude && plugin_allows(plugin_toggles, s.plugin_id.as_deref(), true)
        })
        .map(|s| {
            let token = oauth_tokens.get(&s.id).map(String::as_str);
            (s.name.clone(), s.claude_config_value(token))
        })
        .collect()
}

fn codex_mcp_servers_map(
    servers: &[McpServerEntry],
    plugin_toggles: &BTreeMap<String, (bool, bool)>,
    oauth_tokens: &OauthTokens,
) -> serde_json::Map<String, serde_json::Value> {
    servers
        .iter()
        .filter(|s| {
            s.enabled_codex && plugin_allows(plugin_toggles, s.plugin_id.as_deref(), false)
        })
        .map(|s| {
            let token = oauth_tokens.get(&s.id).map(String::as_str);
            (s.name.clone(), s.codex_config_value(token))
        })
        .collect()
}

impl Entity for McpRegistryModel {
    type Event = ();
}

impl SingletonEntity for McpRegistryModel {}

#[cfg(test)]
mod tests {
    use super::*;

    fn stdio_entry(name: &str) -> McpServerEntry {
        McpServerEntry {
            id: format!("id-{name}"),
            name: name.to_owned(),
            transport: McpTransport::Stdio,
            command: Some("npx".to_owned()),
            args: vec!["-y".to_owned(), "some-mcp".to_owned()],
            url: None,
            env: BTreeMap::from([("TOKEN".to_owned(), "abc".to_owned())]),
            enabled_claude: true,
            enabled_codex: true,
            plugin_id: Some("plug-stdio".to_owned()),
            headers: BTreeMap::new(),
            auth: McpAuth::None,
        }
    }

    fn http_entry(name: &str) -> McpServerEntry {
        McpServerEntry {
            id: format!("id-{name}"),
            name: name.to_owned(),
            transport: McpTransport::Http,
            command: None,
            args: Vec::new(),
            url: Some("https://example.com/mcp".to_owned()),
            env: BTreeMap::new(),
            enabled_claude: true,
            enabled_codex: false,
            plugin_id: Some("plug-http".to_owned()),
            headers: BTreeMap::new(),
            auth: McpAuth::None,
        }
    }

    #[test]
    fn claude_config_renders_stdio_and_http() {
        let servers = vec![stdio_entry("local"), http_entry("remote")];
        let map = claude_mcp_servers_map(&servers, &BTreeMap::new(), &OauthTokens::new());
        assert_eq!(
            serde_json::Value::Object(map),
            serde_json::json!({
                "local": {
                    "type": "stdio",
                    "command": "npx",
                    "args": ["-y", "some-mcp"],
                    "env": { "TOKEN": "abc" },
                },
                "remote": {
                    "type": "http",
                    "url": "https://example.com/mcp",
                },
            })
        );
    }

    #[test]
    fn codex_config_respects_enable_bits_and_shape() {
        // `remote` has enabled_codex = false, so only `local` shows up, in
        // config.toml-override shape (no "type" discriminator).
        let servers = vec![stdio_entry("local"), http_entry("remote")];
        let map = codex_mcp_servers_map(&servers, &BTreeMap::new(), &OauthTokens::new());
        assert_eq!(
            serde_json::Value::Object(map),
            serde_json::json!({
                "local": {
                    "command": "npx",
                    "args": ["-y", "some-mcp"],
                    "env": { "TOKEN": "abc" },
                },
            })
        );
    }

    #[test]
    fn disabled_plugin_drops_its_servers_from_both_configs() {
        // `local` belongs to plug-stdio, which is Claude-disabled; `remote`
        // belongs to plug-http, still Claude-enabled. The component bit stays
        // true — only the plugin AND changes the output.
        let servers = vec![stdio_entry("local"), http_entry("remote")];
        let toggles = BTreeMap::from([
            ("plug-stdio".to_owned(), (false, true)),
            ("plug-http".to_owned(), (true, true)),
        ]);
        let claude = claude_mcp_servers_map(&servers, &toggles, &OauthTokens::new());
        assert_eq!(
            claude.keys().collect::<Vec<_>>(),
            vec![&"remote".to_owned()]
        );
        // Codex: `local`'s plugin allows Codex, `remote` is Codex-disabled at
        // the component level.
        let codex = codex_mcp_servers_map(&servers, &toggles, &OauthTokens::new());
        assert_eq!(codex.keys().collect::<Vec<_>>(), vec![&"local".to_owned()]);

        // Fully-disabled plugin -> empty config (None from the JSON helpers).
        let all_off = BTreeMap::from([
            ("plug-stdio".to_owned(), (false, false)),
            ("plug-http".to_owned(), (false, false)),
        ]);
        let model = McpRegistryModel {
            servers: servers.clone(),
        };
        assert_eq!(model.claude_mcp_config_json(&all_off, &OauthTokens::new()), None);
        assert_eq!(model.codex_config_overrides(&all_off, &OauthTokens::new()), None);
    }

    #[test]
    fn entries_without_plugins_fail_open() {
        let mut entry = stdio_entry("local");
        entry.plugin_id = None;
        let map = claude_mcp_servers_map(&[entry], &BTreeMap::new(), &OauthTokens::new());
        assert!(map.contains_key("local"));
    }

    #[test]
    fn claude_http_gets_headers_and_the_oauth_bearer() {
        // twarp 24a: static headers ride along verbatim; an OAuth entry gains
        // an Authorization header only once a token is resolved.
        let mut entry = http_entry("remote");
        entry.headers = BTreeMap::from([("X-Api-Key".to_owned(), "secret".to_owned())]);
        entry.auth = McpAuth::Oauth;

        let no_token = claude_mcp_servers_map(
            std::slice::from_ref(&entry),
            &BTreeMap::new(),
            &OauthTokens::new(),
        );
        assert_eq!(
            no_token["remote"]["headers"],
            serde_json::json!({ "X-Api-Key": "secret" })
        );

        let tokens = OauthTokens::from([("id-remote".to_owned(), "tok-123".to_owned())]);
        let with_token =
            claude_mcp_servers_map(std::slice::from_ref(&entry), &BTreeMap::new(), &tokens);
        assert_eq!(
            with_token["remote"]["headers"],
            serde_json::json!({ "Authorization": "Bearer tok-123", "X-Api-Key": "secret" })
        );
    }

    #[test]
    fn a_typed_authorization_header_beats_the_cached_token() {
        // Otherwise a stale cached token could shadow the credential the user
        // just pasted in.
        let mut entry = http_entry("remote");
        entry.auth = McpAuth::Oauth;
        entry.headers = BTreeMap::from([(
            "Authorization".to_owned(),
            "Bearer typed-by-hand".to_owned(),
        )]);
        let headers = entry.resolved_headers(Some("cached"));
        assert_eq!(headers["Authorization"], "Bearer typed-by-hand");
    }

    #[test]
    fn codex_maps_bearer_to_bearer_token_and_drops_the_rest() {
        // Codex's mcp_servers config has no header map, only bearer_token.
        let mut entry = http_entry("remote");
        entry.enabled_codex = true;
        entry.headers = BTreeMap::from([
            ("authorization".to_owned(), "bearer tok-123".to_owned()),
            ("X-Api-Key".to_owned(), "secret".to_owned()),
        ]);
        let map = codex_mcp_servers_map(
            std::slice::from_ref(&entry),
            &BTreeMap::new(),
            &OauthTokens::new(),
        );
        assert_eq!(
            map["remote"],
            serde_json::json!({
                "url": "https://example.com/mcp",
                "bearer_token": "tok-123",
            })
        );
        assert!(entry.has_codex_unsupported_headers());
    }

    #[test]
    fn bearer_only_headers_are_fully_expressible_in_codex() {
        let mut entry = http_entry("remote");
        entry.headers =
            BTreeMap::from([("Authorization".to_owned(), "Bearer tok".to_owned())]);
        assert!(!entry.has_codex_unsupported_headers());
        // An empty or non-bearer Authorization is not a token.
        assert!(!is_bearer_authorization("Authorization", "Bearer   "));
        assert!(!is_bearer_authorization("Authorization", "Basic abc"));
        assert!(is_bearer_authorization("AUTHORIZATION", "BEARER abc"));
    }

    #[test]
    fn stdio_servers_never_get_headers() {
        // Headers are an HTTP concept; a local process inherits env instead.
        let mut entry = stdio_entry("local");
        entry.headers = BTreeMap::from([("X-Api-Key".to_owned(), "secret".to_owned())]);
        let claude = claude_mcp_servers_map(
            std::slice::from_ref(&entry),
            &BTreeMap::new(),
            &OauthTokens::new(),
        );
        assert!(claude["local"].get("headers").is_none());
        assert!(!entry.has_codex_unsupported_headers());
    }

    #[test]
    fn pre_24a_rows_migrate_to_no_auth() {
        // A NULL auth column (and an unrecognized value) must read as None so
        // URL-with-embedded-key servers keep working untouched.
        let mut row = http_entry("remote").to_persisted();
        row.auth = None;
        row.headers = None;
        let entry = McpServerEntry::from_persisted(row.clone()).unwrap();
        assert_eq!(entry.auth, McpAuth::None);
        assert!(entry.headers.is_empty());

        row.auth = Some("from-the-future".to_owned());
        assert_eq!(
            McpServerEntry::from_persisted(row).unwrap().auth,
            McpAuth::None
        );
    }

    #[test]
    fn persisted_roundtrip() {
        let entry = stdio_entry("local");
        let back = McpServerEntry::from_persisted(entry.to_persisted()).unwrap();
        assert_eq!(entry, back);

        // twarp 24a: headers + auth survive the round trip too.
        let mut entry = http_entry("remote");
        entry.headers = BTreeMap::from([("X-Api-Key".to_owned(), "secret".to_owned())]);
        entry.auth = McpAuth::Headers;
        let back = McpServerEntry::from_persisted(entry.to_persisted()).unwrap();
        assert_eq!(entry, back);
    }
}
