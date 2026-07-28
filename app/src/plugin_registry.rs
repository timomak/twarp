//! twarp 23a: the plugin registry — a named grouping layer above the
//! MCP-server registry ([`crate::mcp_registry`]) and the shared-skills store
//! ([`crate::skills_store`]).
//!
//! A plugin is a persisted bundle (name, description, per-provider toggles)
//! of N MCP servers and/or N skills. The underlying stores stay authoritative
//! for their components; membership is recorded via the nullable `plugin_id`
//! columns on `mcp_servers` / `shared_skills`. Effective enablement of a
//! component = its own toggle AND its owning plugin's toggle — the plugin
//! toggle "remembers" component state because component bits are never
//! rewritten when the plugin toggle flips.
//!
//! On load, orphan components (rows with `plugin_id` NULL, from a pre-23
//! build) are migrated losslessly into single-component plugins; see
//! [`migrate_orphans`].

use std::collections::BTreeMap;

use twarpui::{Entity, ModelContext, SingletonEntity};

use crate::persistence::{
    ModelEvent, PersistedMcpServer, PersistedPlugin, PersistedSharedSkill,
};
use crate::GlobalResourceHandlesProvider;

/// One plugin, as shown on the Plugins page.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PluginEntry {
    /// UUID string, stable across edits.
    pub id: String,
    /// Unique, user-visible.
    pub name: String,
    pub description: String,
    pub enabled_claude: bool,
    pub enabled_codex: bool,
    /// Member MCP servers ([`crate::mcp_registry::McpServerEntry::id`]).
    pub server_ids: Vec<String>,
    /// Member skills ([`crate::skills_store::SkillEntry::name`]).
    pub skill_names: Vec<String>,
}

impl PluginEntry {
    fn from_persisted(row: PersistedPlugin) -> Self {
        Self {
            id: row.id,
            name: row.name,
            description: row.description,
            enabled_claude: row.enabled_claude,
            enabled_codex: row.enabled_codex,
            server_ids: Vec::new(),
            skill_names: Vec::new(),
        }
    }

    fn to_persisted(&self) -> PersistedPlugin {
        PersistedPlugin {
            id: self.id.clone(),
            name: self.name.clone(),
            description: self.description.clone(),
            enabled_claude: self.enabled_claude,
            enabled_codex: self.enabled_codex,
        }
    }

    /// Component summary shown on the plugin card: `2 servers · 1 skill`.
    pub fn component_summary(&self) -> String {
        let mut parts = Vec::new();
        match self.server_ids.len() {
            0 => {}
            1 => parts.push("1 server".to_owned()),
            n => parts.push(format!("{n} servers")),
        }
        match self.skill_names.len() {
            0 => {}
            1 => parts.push("1 skill".to_owned()),
            n => parts.push(format!("{n} skills")),
        }
        if parts.is_empty() {
            "empty".to_owned()
        } else {
            parts.join(" · ")
        }
    }
}

/// `base`, or the first `base-2`, `base-3`, … not in `taken`.
pub fn unique_plugin_name(taken: &[String], base: &str) -> String {
    if !taken.iter().any(|n| n == base) {
        return base.to_owned();
    }
    (2..)
        .map(|i| format!("{base}-{i}"))
        .find(|name| !taken.iter().any(|n| n == name))
        .expect("unbounded suffix search always terminates")
}

/// What [`migrate_orphans`] produced. `changed` is false when every component
/// already had an owning plugin (re-running is a no-op).
pub struct MigrationResult {
    pub plugins: Vec<PersistedPlugin>,
    pub servers: Vec<PersistedMcpServer>,
    pub skills: Vec<PersistedSharedSkill>,
    pub changed: bool,
}

/// twarp 23a migration: adopt every orphan component (server / skill row with
/// `plugin_id` NULL, or pointing at a plugin that no longer exists) into a
/// freshly created single-component plugin. The plugin inherits the
/// component's name (deduped with a `-2` / `-3` suffix on collision) and its
/// per-provider toggles; the component's own toggles reset to enabled since
/// the plugin toggle now carries the old value. Idempotent.
pub fn migrate_orphans(
    plugins: Vec<PersistedPlugin>,
    servers: Vec<PersistedMcpServer>,
    skills: Vec<PersistedSharedSkill>,
) -> MigrationResult {
    let mut plugins = plugins;
    let mut servers = servers;
    let mut skills = skills;
    let mut changed = false;

    let plugin_exists =
        |plugins: &[PersistedPlugin], id: &Option<String>| match id {
            Some(id) => plugins.iter().any(|p| &p.id == id),
            None => false,
        };

    let mut taken: Vec<String> = plugins.iter().map(|p| p.name.clone()).collect();

    for server in &mut servers {
        if plugin_exists(&plugins, &server.plugin_id) {
            continue;
        }
        let name = unique_plugin_name(&taken, &server.name);
        taken.push(name.clone());
        let id = uuid::Uuid::new_v4().to_string();
        plugins.push(PersistedPlugin {
            id: id.clone(),
            name,
            description: String::new(),
            enabled_claude: server.enabled_claude,
            enabled_codex: server.enabled_codex,
        });
        server.plugin_id = Some(id);
        server.enabled_claude = true;
        server.enabled_codex = true;
        changed = true;
    }

    for skill in &mut skills {
        if plugin_exists(&plugins, &skill.plugin_id) {
            continue;
        }
        let name = unique_plugin_name(&taken, &skill.name);
        taken.push(name.clone());
        let id = uuid::Uuid::new_v4().to_string();
        plugins.push(PersistedPlugin {
            id: id.clone(),
            name,
            description: String::new(),
            enabled_claude: skill.enabled_claude,
            enabled_codex: skill.enabled_codex,
        });
        skill.plugin_id = Some(id);
        skill.enabled_claude = true;
        skill.enabled_codex = true;
        changed = true;
    }

    plugins.sort_by(|a, b| a.name.cmp(&b.name));
    MigrationResult {
        plugins,
        servers,
        skills,
        changed,
    }
}

/// Singleton holding the plugin registry; loaded from SQLite at startup
/// (post-migration), persisted on every mutation by replacing the whole
/// (small) table.
pub struct PluginRegistryModel {
    plugins: Vec<PluginEntry>,
}

impl PluginRegistryModel {
    /// Build the in-memory registry from the (already migrated) persisted
    /// rows, deriving component membership from the components' `plugin_id`s.
    pub fn new(
        persisted: Vec<PersistedPlugin>,
        servers: &[PersistedMcpServer],
        skills: &[PersistedSharedSkill],
    ) -> Self {
        let mut plugins: Vec<PluginEntry> = persisted
            .into_iter()
            .map(PluginEntry::from_persisted)
            .collect();
        for server in servers {
            if let Some(plugin) = server
                .plugin_id
                .as_deref()
                .and_then(|id| plugins.iter_mut().find(|p| p.id == id))
            {
                plugin.server_ids.push(server.id.clone());
            }
        }
        for skill in skills {
            if let Some(plugin) = skill
                .plugin_id
                .as_deref()
                .and_then(|id| plugins.iter_mut().find(|p| p.id == id))
            {
                plugin.skill_names.push(skill.name.clone());
            }
        }
        Self { plugins }
    }

    pub fn plugins(&self) -> &[PluginEntry] {
        &self.plugins
    }

    pub fn get(&self, id: &str) -> Option<&PluginEntry> {
        self.plugins.iter().find(|p| p.id == id)
    }

    /// Whether `name` is already used by a plugin other than `except_id`.
    pub fn name_taken(&self, name: &str, except_id: Option<&str>) -> bool {
        self.plugins
            .iter()
            .any(|p| p.name == name && Some(p.id.as_str()) != except_id)
    }

    /// `base`, or the first `base-2`, `base-3`, … not already registered.
    pub fn unique_name(&self, base: &str) -> String {
        let taken: Vec<String> = self.plugins.iter().map(|p| p.name.clone()).collect();
        unique_plugin_name(&taken, base)
    }

    /// Insert or (matched by id) replace a plugin, then persist.
    pub fn upsert(&mut self, entry: PluginEntry, ctx: &mut ModelContext<Self>) {
        if let Some(existing) = self.plugins.iter_mut().find(|p| p.id == entry.id) {
            *existing = entry;
        } else {
            self.plugins.push(entry);
        }
        self.plugins.sort_by(|a, b| a.name.cmp(&b.name));
        self.persist(ctx);
    }

    pub fn delete(&mut self, id: &str, ctx: &mut ModelContext<Self>) {
        let before = self.plugins.len();
        self.plugins.retain(|p| p.id != id);
        if self.plugins.len() != before {
            self.persist(ctx);
        }
    }

    /// Flip one provider-enable bit on a plugin, then persist. Component
    /// toggles are untouched (the cascade is an AND at read time).
    pub fn toggle_enabled(&mut self, id: &str, claude: bool, ctx: &mut ModelContext<Self>) {
        let Some(plugin) = self.plugins.iter_mut().find(|p| p.id == id) else {
            return;
        };
        if claude {
            plugin.enabled_claude = !plugin.enabled_claude;
        } else {
            plugin.enabled_codex = !plugin.enabled_codex;
        }
        self.persist(ctx);
    }

    /// Plugin id -> (enabled_claude, enabled_codex), for gating the MCP
    /// injection paths. Components with no / a dangling plugin are treated as
    /// enabled (fail open; the next load's migration adopts them).
    pub fn provider_toggles_by_id(&self) -> BTreeMap<String, (bool, bool)> {
        self.plugins
            .iter()
            .map(|p| (p.id.clone(), (p.enabled_claude, p.enabled_codex)))
            .collect()
    }

    /// Skill name -> owning plugin's (enabled_claude, enabled_codex), for
    /// gating the skills materializer. Skills owned by no plugin are absent
    /// (treated as enabled by the caller).
    pub fn skill_plugin_toggles(&self) -> BTreeMap<String, (bool, bool)> {
        let mut map = BTreeMap::new();
        for plugin in &self.plugins {
            for name in &plugin.skill_names {
                map.insert(name.clone(), (plugin.enabled_claude, plugin.enabled_codex));
            }
        }
        map
    }

    /// The plugin owning the given skill, if any.
    pub fn plugin_of_skill(&self, skill_name: &str) -> Option<&PluginEntry> {
        self.plugins
            .iter()
            .find(|p| p.skill_names.iter().any(|n| n == skill_name))
    }

    /// The plugin owning the given server, if any.
    pub fn plugin_of_server(&self, server_id: &str) -> Option<&PluginEntry> {
        self.plugins
            .iter()
            .find(|p| p.server_ids.iter().any(|i| i == server_id))
    }

    fn persist(&self, ctx: &mut ModelContext<Self>) {
        ctx.notify();
        let handles = GlobalResourceHandlesProvider::as_ref(ctx).get();
        if let Some(sender) = &handles.model_event_sender {
            let event = ModelEvent::ReplacePlugins {
                plugins: self.plugins.iter().map(PluginEntry::to_persisted).collect(),
            };
            if let Err(err) = sender.send(event) {
                log::error!("Failed to persist plugin registry: {err}");
            }
        }
    }
}

impl Entity for PluginRegistryModel {
    type Event = ();
}

impl SingletonEntity for PluginRegistryModel {}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(name: &str, claude: bool, codex: bool) -> PersistedMcpServer {
        PersistedMcpServer {
            id: format!("srv-{name}"),
            name: name.to_owned(),
            transport: "stdio".to_owned(),
            command: Some("npx".to_owned()),
            enabled_claude: claude,
            enabled_codex: codex,
            ..Default::default()
        }
    }

    fn skill(name: &str, claude: bool, codex: bool) -> PersistedSharedSkill {
        PersistedSharedSkill {
            name: name.to_owned(),
            enabled_claude: claude,
            enabled_codex: codex,
            plugin_id: None,
        }
    }

    #[test]
    fn migration_adopts_orphans_and_copies_toggles() {
        let result = migrate_orphans(
            Vec::new(),
            vec![server("alpha", true, false)],
            vec![skill("beta", false, true)],
        );
        assert!(result.changed);
        assert_eq!(result.plugins.len(), 2);

        let alpha = result.plugins.iter().find(|p| p.name == "alpha").unwrap();
        assert!(alpha.enabled_claude);
        assert!(!alpha.enabled_codex);
        // Component toggles reset to enabled; the plugin carries the old bits.
        assert_eq!(result.servers[0].plugin_id, Some(alpha.id.clone()));
        assert!(result.servers[0].enabled_claude);
        assert!(result.servers[0].enabled_codex);

        let beta = result.plugins.iter().find(|p| p.name == "beta").unwrap();
        assert!(!beta.enabled_claude);
        assert!(beta.enabled_codex);
        assert_eq!(result.skills[0].plugin_id, Some(beta.id.clone()));
        assert!(result.skills[0].enabled_claude);
        assert!(result.skills[0].enabled_codex);
    }

    #[test]
    fn migration_is_idempotent() {
        let first = migrate_orphans(
            Vec::new(),
            vec![server("alpha", true, true)],
            vec![skill("beta", true, true)],
        );
        assert!(first.changed);
        let second = migrate_orphans(
            first.plugins.clone(),
            first.servers.clone(),
            first.skills.clone(),
        );
        assert!(!second.changed);
        assert_eq!(second.plugins.len(), first.plugins.len());
        assert_eq!(second.servers, first.servers);
        assert_eq!(second.skills, first.skills);
    }

    #[test]
    fn migration_suffixes_name_collisions() {
        // A server and a skill both named `slack` -> `slack`, `slack-2`.
        let result = migrate_orphans(
            Vec::new(),
            vec![server("slack", true, true)],
            vec![skill("slack", true, true)],
        );
        let mut names: Vec<&str> = result.plugins.iter().map(|p| p.name.as_str()).collect();
        names.sort();
        assert_eq!(names, vec!["slack", "slack-2"]);
    }

    #[test]
    fn migration_re_adopts_dangling_plugin_ids() {
        let mut orphan = server("alpha", true, true);
        orphan.plugin_id = Some("gone".to_owned());
        let result = migrate_orphans(Vec::new(), vec![orphan], Vec::new());
        assert!(result.changed);
        assert_eq!(result.plugins.len(), 1);
        assert_eq!(
            result.servers[0].plugin_id,
            Some(result.plugins[0].id.clone())
        );
    }

    #[test]
    fn membership_is_derived_from_component_rows() {
        let migrated = migrate_orphans(
            Vec::new(),
            vec![server("alpha", true, true)],
            vec![skill("beta", true, true)],
        );
        let model =
            PluginRegistryModel::new(migrated.plugins, &migrated.servers, &migrated.skills);
        let alpha = model
            .plugins()
            .iter()
            .find(|p| p.name == "alpha")
            .unwrap();
        assert_eq!(alpha.server_ids, vec!["srv-alpha".to_owned()]);
        assert_eq!(alpha.component_summary(), "1 server");
        let beta = model.plugins().iter().find(|p| p.name == "beta").unwrap();
        assert_eq!(beta.skill_names, vec!["beta".to_owned()]);
        assert_eq!(beta.component_summary(), "1 skill");
        assert_eq!(
            model.skill_plugin_toggles().get("beta"),
            Some(&(true, true))
        );
    }
}
