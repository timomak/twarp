# 18 — Multi-provider agent pane (Codex backend)

**Phase:** not-started (queued after 19 by owner direction, 2026-07-16)
**Spec PR:** [#218](https://github.com/timomak/twarp/pull/218) (authored 2026-07-16, owner session) — [PRODUCT.md](PRODUCT.md) + [TECH.md](TECH.md) are complete; the fleet's `18-agent-providers-spec` item should review/adjust them against the then-current tree (19 will have landed) rather than rewrite, then flip this phase to `impl-pending`.
**Impl PRs:** —

## Scope

Make the Claude pane provider-generic and light up **Codex** as the second backend, behind the pane's existing normalized event boundary (`claude_code::TranscriptEvent`) and feature 16's `CLIAgent`/adapter scaffolding. Integration surface is `codex app-server` v2 (JSON-RPC over stdio). Claude behavior is regression-barred via golden-transcript tests. No visual changes (feature 19 owns the look).

## Sub-phases

- [ ] **18a — driver extraction.** Runtime `AgentDriver` trait (spawn/send/interrupt/answer/parse/sessions/capabilities); typed approval `Decision`; `provider` column on pane persistence (default claude); golden-transcript tests; zero behavior change.
- [ ] **18b — codex driver.** Vendored app-server v2 protocol subset pinned to a minimum CLI version; spawn + initialize handshake; thread start/resume; event mapping to `TranscriptEvent`; `turn/interrupt` with tracked turnId; fixture replay tests; behind the `CodexAgentBackend` feature flag.
- [ ] **18c — approvals & access.** Unified approval card for both protocols with a guaranteed-reply invariant; four-stop Access pill (Read-only / Ask to edit / Edits allowed / Full access) mapped per provider; bypass-flag detection.
- [ ] **18d — entry & light-up.** `codex` terminal trigger + alias expansion; settings-16 Codex adapter enabled (install/login probes, model+effort lists); login-in-a-terminal-split flow; min-version upgrade card; provider-tagged past-sessions sidebar with filter.
- [ ] **18e — capability polish.** Fork via `thread/fork`; provider-shaped usage line (tokens/quota vs cost); steering behind a capability flag; readable provider-error ended states.
- [ ] **18f — in-pane provider switching.** Cursor-style provider control on the composer pill, idle-only; fresh-pane seamless swap; mid-conversation handoff via transcript digest with a visible switch divider; segments persistence + stitched restore; Access-stop remap; owner-directed 2026-07-16.

## Smoke test

See PRODUCT.md `## Smoke test` (per-sub-phase steps; the UX gate reads that section).
