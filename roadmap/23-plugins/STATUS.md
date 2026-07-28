# 23 — Plugins: unify Skills + MCPs

**Phase:** impl-in-review (spec + impl bundled)
**Spec PR:** [#281](https://github.com/timomak/twarp/pull/281)
**Impl PRs:** bundled in [#281](https://github.com/timomak/twarp/pull/281) (23a–23c)

## Scope

Merge the Skills (20c) and MCPs (20b/22) automation pages into one **Plugins**
page where a plugin = metadata + N MCP servers + N skills, with cascading
per-provider toggles. Existing entries migrate losslessly into
single-component plugins; quick-add presets become gallery cards; built-ins
(`twarp-browser`, `twarp-computer-control`) present as first-party plugins.
Owner-directed 2026-07-28 (Codex-app "Plugins" parity discussion).

## Owner-confirmed decisions (2026-07-28)

- Combine the concepts rather than relabel the MCP page — a plain
  "MCPs → Plugins" rename was considered and rejected (skills would have
  stayed a misplaced sibling page).
- Plugin = packaging layer only; no protocol change, injection paths stay.
- Slash-command names (`/add-mcp`, `/open-mcp-servers`) keep their names;
  descriptions and visible labels say "plugin".

## Sub-phases

- [x] **23a — plugin registry + migration** (bundled with 23b per the
      not-independently-testable rule)
- [x] **23b — Plugins page** (replaces Skills + MCPs pages)
- [x] **23c — gallery + renames** (presets as cards, Claude pill, labels)

## Smoke test

_(authored in PRODUCT.md)_

Smoke test (PRODUCT.md steps 1-9) still needs an owner `./script/run` pass.
