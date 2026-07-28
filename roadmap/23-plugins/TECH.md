# 23 — Plugins: unify Skills + MCPs (TECH)

## Approach: grouping layer, not a rewrite

The two underlying stores are healthy and stay authoritative:

- `app/src/mcp_registry.rs` (20b) — `McpServerEntry` rows in the
  `mcp_servers` SQLite table, injected into Claude via inline `--mcp-config`
  and Codex via `mcp_servers` config overrides at spawn.
- `app/src/skills_store.rs` (20c) — `~/.twarp/skills/<name>/SKILL.md` on
  disk + `shared_skills` SQLite toggles, materialized as Claude symlinks and
  Codex prompt files.

A **plugin is a persisted grouping record above both**. Injection and
materialization code paths are untouched except for reading effective
enablement through the plugin layer.

## Data model

New module `app/src/plugin_registry.rs`:

```rust
pub struct PluginEntry {
    pub id: String,          // UUID
    pub name: String,        // unique, user-visible
    pub description: String,
    pub enabled_claude: bool,
    pub enabled_codex: bool,
    // components reference the underlying stores:
    pub server_ids: Vec<String>,   // McpServerEntry.id
    pub skill_names: Vec<String>,  // SkillEntry.name
}
```

Persistence: new `plugins` table (id, name, description, enabled_claude,
enabled_codex) + nullable `plugin_id` columns on `mcp_servers` and
`shared_skills` (follow the `#[sql_name]` / migration patterns from PR #152's
schema notes; schema.rs column names must match the DB).

**Effective enablement** = component toggle AND its plugin's toggle. The
cascade "remembers" component state because component toggles are simply not
rewritten when the plugin toggle flips — only the AND changes.
`McpRegistryModel::claude_mcp_config_json()` / `codex_config_overrides()` and
`SkillsStoreModel`'s materializer take effective enablement as their filter.

## Migration

In the persistence load path (where `PersistedMcpServer` /
`PersistedSharedSkill` are read): any server/skill row with `plugin_id IS
NULL` gets a plugin auto-created (same name, description from skill
frontmatter where available, provider toggles copied; component toggles reset
to enabled since the plugin toggle now carries the old value). Idempotent —
runs only for orphan rows, so re-running is a no-op. No data is deleted;
downgrade keeps working because old builds ignore the new table/columns.

Name collisions during migration (a server and a skill both named `slack`)
resolve with the existing `unique_name` suffixing (`slack`, `slack-2`).

## UI

- `AutomationPage`: replace `Skills` and `Mcps` variants with `Plugins`.
  **Persistence-format note:** `from_persistence_str` must keep accepting
  `"skills"` and `"mcps"` (mapping both to `Plugins`) so restored panes from
  old snapshots don't dead-end; `as_persistence_str` emits `"plugins"`.
- New `app/src/automation/plugins_page.rs` composed largely from the existing
  `mcps_page.rs` (row chrome, inline editor, presets, switches) and
  `skills_page.rs` (skill rows, conflict badges, adopt flow). The old two
  pages are deleted once the merge lands (sub-phase 23b), not kept as
  redirects.
- Sidebar: one `render_sidebar_action_with_count` row; count =
  `plugin_registry.plugins().len()`. `WorkspaceAction::ShowSkills`/`ShowMcps`
  both open the Plugins page (keep the action variants to avoid churning
  keybinding configs; add `ShowPlugins` as the primary).
- Built-in section: static descriptors for `twarp-browser` /
  `twarp-computer-control` (reuse the existing built-in rows from
  `mcps_page.rs`, restyled as cards).
- Claude popover rename in `claude_code_view.rs` is string-only (the
  standalone `MCP · N` pill was folded into the session chip's menu by PR
  #272, so only the popover header/empty-state strings change).
- Editor semantics: no attach-existing-skill picker (post-migration every
  skill has a plugin); adopt creates a single-skill plugin immediately;
  components removed during edit spin out into single-component plugins at
  save; deleting a plugin deletes its member servers/skills. Sub-form
  actions are keyed by stable UUIDs, not indices, so Remove can't stale
  sibling callbacks.
- Skills materializer: toggle rows now carry `plugin_id`; `apply_scan`
  backfills toggle rows for on-disk skills so the next load's migration
  adopts them (one-launch-late, fail-open in the interim).
- Follow `warp-ui-guidelines` + tokens (`type_ramp`, `spacing`, `radius`);
  no new colors outside the theme.

## Risks / gotchas

- **Skills adopt flow** (real dir in `~/.claude/skills` → store) must keep
  working; adopted skills land as single-skill plugins.
- **Empty-stub layout trap**: any new icon in cards must be wrapped in
  `ConstrainedBox` (see memory: bare `Icon` lays out at constraint.max).
- **Pane persistence**: `automation_panes.page` values from old snapshots
  (see UI note above) — cover with a unit test on
  `from_persistence_str("skills"/"mcps")`.
- The hidden `SettingsSection::MCPServers` stub and its Display/FromStr
  round-trip tests are left untouched.

## Sub-phases

- **23a — plugin registry + migration.** `plugin_registry.rs`, schema
  additions, orphan-row migration, effective-enablement plumbed into both
  injection paths. Unit tests: migration idempotence, cascade AND,
  config-JSON filtering.
- **23b — Plugins page.** New page replacing Skills + MCPs (sidebar row,
  pane routing, persistence aliases), card list + inline multi-component
  editor, built-in section, delete/edit flows.
- **23c — gallery + renames.** Presets as quick-add cards (record shape
  allows bundled skills), Claude pill/popover rename, palette/slash/menu
  label updates.

23a is not independently smoke-testable end-to-end (pure data layer), so per
the bundling rule it ships with 23b in one PR; 23c can ride along if review
size stays sane.

## Validation

- `rust-unit-tests` on plugin_registry (migration, cascade, JSON output).
- Smoke test in PRODUCT.md against a `./script/run` build, including the
  cross-provider toggle checks (steps 5–7).
