# 18 — Multi-provider agent pane (Codex backend)

**Phase:** not-started (queued after 19 by owner direction, 2026-07-16)
**Spec PR:** —
**Impl PRs:** —

## Scope (direction summary — spec to be written when 18 activates)

Make the Claude pane provider-generic and light up **Codex** as the second backend, behind the pane's existing normalized event boundary (`claude_code::TranscriptEvent`) and feature 16's `CLIAgent`/adapter scaffolding.

- **18a** — runtime `AgentDriver` trait extraction (spawn/send/interrupt/approve/parse behind the trait; `provider` column on pane persistence; golden-transcript tests; zero behavior change).
- **18b** — Codex driver via `codex app-server` v2 (JSON-RPC over stdio; vendored protocol types pinned to a minimum CLI version; thread/turn/item mapping; `turn/interrupt` with tracked turnId), behind a feature flag.
- **18c** — approvals + unified four-stop **Access** pill (Read-only / Ask to edit / Edits allowed / Full access) mapped per provider; one approval-card anatomy for both protocols.
- **18d** — entry points & light-up: `codex` terminal trigger, provider-tagged sessions sidebar, settings-16 adapter enabled, auth detection + login-in-a-terminal-tab.
- **18e** — capability polish: fork (`thread/fork`), steering, tokens-vs-cost usage line, quota display.

Key research (2026-07-16) is recorded in the collaborator memory (`twarp-multiprovider-design-direction`): protocol event mapping, auth/model/sandbox facts, coupling map with path:line seams.
