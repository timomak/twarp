# 23 — Plugins: unify Skills + MCPs (PRODUCT)

## Problem

twarp's Automation sidebar exposes **Skills** (20c) and **MCPs** (20b/22) as two
sibling pages, but they are two halves of one thing: an MCP server is a
*capability* (tools the agent can call) and a skill is the *knowledge* of when
and how to use it. Shipping them separately means:

- A user wiring up "Slack for my agents" has to visit two pages and mentally
  associate a server with a skill.
- Raw MCP servers often underperform without an accompanying skill, and skills
  that orchestrate a server are inert without it — but nothing in the UI
  expresses that pairing.
- Built-ins like `twarp-browser` are already de-facto bundles (server + usage
  instructions) but render as a bare read-only server row.

The Codex/ChatGPT app solved this with **Plugins**: a product wrapper (name,
icon, description, sample prompts) around N MCP servers + N skills with
per-component toggles. There is no new protocol underneath — it's packaging.

## Goal

Replace the Skills and MCPs pages with a single **Plugins** page where each
plugin is a named bundle of MCP servers and/or skills, with plugin-level and
component-level per-provider (Claude / Codex) toggles. Existing servers and
skills migrate losslessly into single-component plugins.

## Non-goals (explicit)

- **No marketplace / remote gallery.** Quick-add presets stay compiled-in;
  no network fetch of plugin listings.
- **No new agent-side protocol.** Injection into Claude (`--mcp-config`,
  `~/.claude/skills`) and Codex (`mcp_servers` overrides, `~/.codex/prompts`)
  is unchanged; plugins are a grouping layer above the existing stores.
- **No credential vaulting changes.** Env vars stay plain key=value in the
  server form, as today.
- **No sample-prompt execution.** The detail view may show a description, but
  ChatGPT-style "Try now" prompt chips are out of scope.

## Behavior

### The Plugins page (replaces Skills + MCPs sidebar rows)

- The project sidebar's **Skills** and **MCPs** rows are replaced by one
  **Plugins** row (icon: Dataflow) with a count of plugins. `WorkspaceAction::
  ShowSkills` / `ShowMcps` surfaces (palette entries, slash commands) route to
  the Plugins page.
- The page lists plugins as **cards/rows**: name, description (muted, one
  line), component summary (`2 servers · 1 skill`), Claude/Codex enable
  switches, Edit / Delete.
- Plugin-level provider switches **cascade**: disabling Claude on the plugin
  disables all its components for Claude (component states are remembered and
  restored when re-enabled).
- **First-party plugins** (`twarp-browser`, `twarp-computer-control`) render
  as read-only cards in a "Built-in" section — name, description, provider
  availability — mirroring how ChatGPT presents Computer Use.

### Plugin detail / editor

- Clicking a plugin (or Add plugin) opens the inline expanding detail form
  (no modals, matching the current MCPs editor):
  - **Metadata:** name (required, unique), description (optional).
  - **MCP servers section:** zero or more servers, each with the existing
    transport/command/args/URL/env form and per-provider toggles.
  - **Skills section:** zero or more skills created inline (name +
    description, as on the current Skills page); per-provider toggles;
    conflict badges (existing `claude_conflict` / `codex_conflict` states)
    surface unchanged. There is no "attach existing skill" picker — after
    migration every skill already belongs to a plugin, and adopting a skill
    from `~/.claude/skills` immediately creates a single-skill plugin.
- A plugin must contain at least one component to save.
- Removing a component while editing does not delete or orphan it: at save
  time the removed server/skill spins out into its own single-component
  plugin.
- **Deleting a plugin deletes its components**: member servers leave the
  registry and member skills leave the store (their symlinks/prompt
  artifacts are cleaned up).

### Quick add → gallery

- The existing presets (Slack, Composio, Notion, Linear, Gmail, GitHub,
  Cloudflare) become **plugin gallery cards** in a "Quick add" section:
  label + one-line description. Clicking opens the Add form prefilled as a
  single-server plugin (same prefill behavior as today, unique-name suffixing
  preserved).
- Presets may later ship a bundled skill; the record shape must allow it, but
  no preset is required to include one in this pass.

### Migration (invisible, lossless)

- On first launch after upgrade, every existing MCP-registry entry becomes a
  plugin with that one server, and every shared skill becomes a plugin with
  that one skill. Names, env, and per-provider toggles carry over exactly.
- Edge case: a skill directory that exists on disk but was never toggled
  (no `shared_skills` row) is adopted one launch late — the first scan
  backfills its toggle row and the next load's migration wraps it in a
  plugin. Until then it stays enabled (fail-open), matching pre-23 behavior.
- Agent sessions behave identically before and after migration: the same
  servers are injected, the same skills are materialized.

### Renamed surfaces

- Claude-pane MCP popover (reached via the session chip's menu since PR
  #272 — the standalone `MCP · N` pill no longer exists): header "Plugins",
  empty state "No plugins connected." (contents unchanged — it still lists
  the session's MCP servers, and the claude-CLI hint line stays).
- Command palette / slash commands: `/add-mcp` and `/open-mcp-servers` keep
  their names (muscle memory) but descriptions say "plugin"; menu binding
  labels say "Open Plugins".

## Users & value

- One page answers "what can my agents do and why": capability + knowledge
  per integration, toggleable per provider in one place.
- Built-ins get honest billing as first-party plugins instead of mystery rows.
- Sets up a future gallery/marketplace shape without committing to one.

## Smoke test

Prereq: at least one user MCP server and one shared skill exist from a
previous build. Build twarp (`./script/run`).

1. Sidebar shows a single **Plugins** row (Skills and MCPs rows are gone)
   with a count equal to migrated servers + skills.
2. Open Plugins: each pre-existing server and skill appears as its own
   plugin card with its old name and provider toggles preserved.
3. Built-in section lists `twarp-browser` (and `twarp-computer-control`
   where available) read-only.
4. Add plugin → name "test", add one stdio server and one inline skill →
   Save. Card shows `1 server · 1 skill`.
5. Toggle the plugin's Claude switch off: start a new Claude session — the
   server is absent from the session's plugin popover and `/test` does not
   resolve. Toggle back on: a new session sees both again.
6. Quick add → Slack: form opens prefilled as a single-server plugin; Save
   with a token; new Claude session lists it.
7. Delete the "test" plugin: its skill leaves `~/.claude/skills` and
   `~/.codex/prompts`; new sessions no longer get the server.
8. Claude pane session-chip menu's MCP section header reads "Plugins"
   (and "No plugins connected." when the session has none).
9. Restart twarp: plugins, toggles, and built-ins render identically.
