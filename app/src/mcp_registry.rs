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

    /// The `mcpServers.<name>` value in Claude `--mcp-config` shape.
    fn claude_config_value(&self) -> serde_json::Value {
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
            McpTransport::Http => serde_json::json!({
                "type": "http",
                "url": self.url.clone().unwrap_or_default(),
            }),
        }
    }

    /// The `mcp_servers.<name>` value in Codex `config.toml`-override shape
    /// (passed via the app-server `thread/start` `config` object).
    fn codex_config_value(&self) -> serde_json::Value {
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
            McpTransport::Http => serde_json::json!({
                "url": self.url.clone().unwrap_or_default(),
            }),
        }
    }
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
    /// `{"mcpServers": {...}}` over the Claude-enabled entries, or `None` when
    /// there are none. Built-ins are merged on top by the caller, so they win
    /// name collisions.
    pub fn claude_mcp_config_json(&self) -> Option<String> {
        let servers = claude_mcp_servers_map(&self.servers);
        (!servers.is_empty()).then(|| serde_json::json!({ "mcpServers": servers }).to_string())
    }

    /// Registry contribution to the Codex app-server `thread/start` `config`
    /// overrides: `{"mcp_servers": {...}}` over the Codex-enabled entries, or
    /// `None` when there are none.
    pub fn codex_config_overrides(&self) -> Option<serde_json::Value> {
        let servers = codex_mcp_servers_map(&self.servers);
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

fn claude_mcp_servers_map(
    servers: &[McpServerEntry],
) -> serde_json::Map<String, serde_json::Value> {
    servers
        .iter()
        .filter(|s| s.enabled_claude)
        .map(|s| (s.name.clone(), s.claude_config_value()))
        .collect()
}

fn codex_mcp_servers_map(servers: &[McpServerEntry]) -> serde_json::Map<String, serde_json::Value> {
    servers
        .iter()
        .filter(|s| s.enabled_codex)
        .map(|s| (s.name.clone(), s.codex_config_value()))
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
        }
    }

    #[test]
    fn claude_config_renders_stdio_and_http() {
        let servers = vec![stdio_entry("local"), http_entry("remote")];
        let map = claude_mcp_servers_map(&servers);
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
        let map = codex_mcp_servers_map(&servers);
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
    fn persisted_roundtrip() {
        let entry = stdio_entry("local");
        let back = McpServerEntry::from_persisted(entry.to_persisted()).unwrap();
        assert_eq!(entry, back);
    }
}
