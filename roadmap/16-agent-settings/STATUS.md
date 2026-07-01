# 16 — Agent settings — STATUS

**Phase:** **spec-in-review** — PRODUCT.md + TECH.md drafted 2026-07-01 (expanded same day per owner Q&A), spec PR pending. No implementation yet.

**Owner-confirmed decisions (2026-07-01):**
- Provider scope: **independent per action** (chat / terminal-suggest / reply-suggest each get their own provider+model+effort).
- API key: **keychain in Phase 1** (subscription/local-CLI auth stays the zero-config default).
- Config UI: **unified** — the Chat matrix row IS the new-chat model/effort/mode; no separate "defaults" block.
- Defaults authority: **authoritative** — the Chat row overwrites the pills' last-used memory for new panes.
- Backends: **show Codex/Gemini disabled** ("coming soon").
- Scope: **config + BOTH suggestion generators** — feature 16 also builds reply ghost-text (16e) and terminal AI suggestions (16f).

## Scope

A new **Agent** settings page (backend selector, local-auth reuse or API key via OS keychain, per-action model matrix with the Chat row authoritative for new panes) **plus** the two suggestion consumers that read it. Phase 1 is **Claude-only chat backend with a multi-provider, capability-aware schema**.

## Sub-phases (see TECH.md §Sub-phase plan)

- [ ] **16a** — Agent page scaffold + unified authoritative Chat config seeding the spawn seam.
- [ ] **16b** — Auth status probe + API-key storage in the OS keychain.
- [ ] **16c** — Per-action matrix (terminal + reply rows) + enable toggles.
- [ ] **16d** — Provider-abstraction hardening (capability model + adapter seam; Claude-only impl).
- [ ] **16e** — Chat reply suggestions (ghost text in the composer) via `SuggestionProvider`.
- [ ] **16f** — Terminal AI command suggestions (fallback below instant history) via `SuggestionProvider`.

## Key constraints carried into the specs

- **Subscription generation is the heavy path** (resident/`-p` `claude`, debounced) — an API-key provider is the low-latency path and recommended default for suggestion rows. This is why the matrix allows a different provider per action.
- Suggestion generators are **off by default**, each behind its own enable toggle; best-effort (unauth/failure → no suggestion, never a hang), never auto-send/auto-run.

## Dependencies

- **Codex/Gemini adapters** are a Phase-2 follow-on; this feature only requires the schema + disabled selector entries.

## Why a new feature (not 07 sub-phase)

Feature 07 is merged (phases 1+2). This spans the settings surface, provider abstraction, keychain, terminal, and two generators — well beyond the Claude pane — so it is its own roadmap feature.
