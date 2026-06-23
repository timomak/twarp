# 13 — MCP viewer in the Claude pane (PRODUCT)

## Problem

The Claude pane runs a local headless `claude` subprocess that can use MCP (Model
Context Protocol) servers, but the pane gives the user **no visibility** into which
MCP servers are connected to the current session or what tools they expose. The old
MCP settings page was deleted in feature 02 (`mcp_servers_page.rs` is an empty
placeholder). Today a user must drop to a separate terminal and run `claude mcp list`
to know what's wired up. When Claude invokes an MCP tool, the pane renders it as a
generic tool card (`mcp__github__create_issue` → `github · create_issue`) but never
shows the server inventory.

## Goal

Give the user **read-only** visibility into the MCP servers available to the current
Claude session, surfaced inside the pane next to the existing composer controls. No
configuration this pass — adding, editing, removing, enabling, or disabling servers
stays in the `claude` CLI.

## Non-goals (explicit)

- **No management.** No add / edit / remove / enable / disable. The popover is a
  viewer, not an editor. (Possible future sub-phase.)
- **No reviving the settings page.** This lives in the Claude pane, not
  `settings_view/mcp_servers_page.rs`.
- **No new subprocess calls.** We do not shell out to `claude mcp list` separately;
  the data comes from the session the pane already drives.
- **No credential/secret display.** Server names, status, and tool names only — never
  env vars, tokens, or command-line args.

## Users & value

- A user about to ask Claude to do something MCP-backed (file an issue, query Linear,
  hit a database) can confirm at a glance that the relevant server is connected and
  what tools it offers — without leaving the pane or guessing from tool-card names.
- A user debugging "why didn't Claude use my MCP server" can see whether it connected
  (`status`) at all.

## Behavior

### The MCP pill

- A new control pill labeled **MCP** sits in the composer controls row, alongside the
  existing permission / model / effort pills and the context chip.
- The pill shows a **count badge** of connected servers, e.g. `MCP · 3`. When zero
  servers are configured for the session, the pill still renders but reads `MCP · 0`
  (so the feature is discoverable even with nothing connected).
- Clicking the pill toggles an **MCP popover** open/closed, mirroring the
  permission/model dropdowns (opens just above the pills row). Clicking the pill again,
  or opening another dropdown, closes it.
- Unlike the permission/model/effort pills, the MCP pill is **always clickable, even
  while streaming** (it's read-only and safe), matching how the context chip behaves.

### The MCP popover

- Header row: `MCP servers`.
- One row per server, each showing:
  - **Server name** (e.g. `github`, `linear`).
  - **Connection status** from the session init event, rendered as a small colored
    indicator + label: `connected` (green), `failed` (red), `pending` (muted), or the
    raw status string if it's something else. Status is best-effort — if the init event
    omits status for a server, show `connected` (it was listed) with a muted style.
  - **Tool count** for that server, derived from MCP tools observed in this session,
    e.g. `4 tools`. If no tools have been observed yet, show `tools unknown` (muted) —
    see the data note below.
- Expanding a server row (click) reveals its **tool list** — the bare tool names with
  the `mcp__server__` prefix stripped (e.g. `create_issue`, `list_issues`). Only one
  server is expanded at a time; clicking an expanded row collapses it. Tools are listed
  in first-seen order.
- **Empty state.** If the session has no MCP servers, the popover shows a single muted
  line: `No MCP servers connected.` followed by a hint: `Configure with the claude CLI
  (claude mcp add).`

### Data sources & freshness

- **Server list + status** come from the `claude` stream-json **session-init/system
  event** (`mcp_servers: [{ name, status }]`). This is captured when the session
  starts (and on resume, the init event replays).
- **Per-server tools** are **derived from observed tool calls** in the session. The
  init event does *not* enumerate tools, so the tool list is built incrementally: each
  `mcp__<server>__<tool>` tool_use seen in the transcript adds `<tool>` to that
  server's set. This means tool counts may start at `tools unknown` and grow as Claude
  actually uses the servers during the session. This is an accepted limitation for the
  read-only pass and must be reflected honestly in the UI (the `tools unknown` state),
  not faked.
- The popover reflects the **current session only**. Switching/resuming sessions
  refreshes from that session's init event.

## Smoke test

Prereq: have at least one MCP server configured for the `claude` CLI (e.g.
`claude mcp add` a simple server, or use an existing one). Build twarp
(`./script/run`).

1. Open twarp. In a terminal tab, run `claude` to open the Claude pane.
2. The composer controls row shows an **MCP** pill with a count, e.g. `MCP · 1`,
   alongside the permission / model / effort pills.
3. Click the **MCP** pill. A popover opens above the pills listing each configured
   server by name with a status indicator. A connected server shows a green indicator.
4. Each server row shows a tool count or `tools unknown` if Claude hasn't called any of
   its tools yet.
5. Ask Claude to do something that uses an MCP tool (e.g. an `mcp__<server>__<tool>`
   call). After the call completes, reopen the popover: that server's tool count
   increments and the tool name appears when the row is expanded.
6. Click a server row to expand it; its tool list shows bare tool names (prefix
   stripped). Click again to collapse.
7. Click the MCP pill again (or open the model dropdown): the MCP popover closes.
8. While Claude is **streaming** a response, the MCP pill is still clickable and the
   popover still opens (read-only), unlike the permission/model/effort pills which are
   disabled mid-stream.
9. Start a session with **no** MCP servers configured: the pill reads `MCP · 0` and the
   popover shows `No MCP servers connected.` with the CLI hint.
10. Restart twarp and resume the session: the MCP server list repopulates from the
    replayed init event.
