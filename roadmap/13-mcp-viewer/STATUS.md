# 13 — MCP viewer in Claude pane

**Phase:** merged
**Spec PR:** [#91](https://github.com/timomak/twarp/pull/91)
**Impl PRs:** 13a [#95](https://github.com/timomak/twarp/pull/95) (merged)

## Scope

Read-only visibility into the MCP servers available to the local `claude` CLI, surfaced inside the Claude pane. Owner-requested follow-on to feature 07.

- A composer control (pill) next to the existing permission / model / effort controls that opens an **MCP popover**.
- The popover lists the **connected MCP servers** and, per server, its **tools** (and connection status where available).
- **Read-only this pass.** No add / edit / remove / enable-disable. Management stays in the `claude` CLI (`claude mcp add|remove|list`, `~/.claude.json`, project `.mcp.json`).

## Owner-confirmed decisions (2026-06-23)

- **New feature, not a feature-07 sub-phase** — keeps the merged feature 07 closed.
- **View-only** scope for the first pass; enable/disable and full management are explicit non-goals (possible later sub-phases).
- **Placement:** composer pill/popover in the Claude pane, consistent with the existing permission/model/effort chrome — not the removed settings page.
- **Pulled ahead of 09-rebrand** by owner direction.

## Sub-phases

_(to be finalized in TECH.md)_

- [x] **13a — MCP viewer.** Source the server/tool list, add the composer pill + popover, render servers and their tools read-only.

## Smoke test

_(authored in PRODUCT.md)_
