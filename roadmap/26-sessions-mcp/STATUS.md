# Feature 26 — Sessions & Projects MCP: status

Phase: **26d implemented**

- 26a spec — PRODUCT.md + TECH.md written 2026-07-31, PR pending review
- 26b read path (`list_sessions`, `get_transcript`, `list_projects` + SessionRegistry) — impl done 2026-07-31 (`app/src/sessions_mcp/`), unit tests green; in-app smoke pending
- 26c events (`watch_session`, `wait_for_completion` + registry broadcast channels) — impl done 2026-07-31 (`app/src/sessions_mcp/events.rs` + registry/bridge), unit tests green; in-app smoke pending
- 26d spawning + projects + external token-gated listener (`create_chat`, `create_project`, settings) — impl done 2026-07-31 (spawn reservation in the registry, provenance chip + persisted origin, fixed-port token-gated listener + Agents-page rows), unit tests green; in-app smoke pending
- 26e Codex built-in-server injection parity — not started
- 26f fleet UX-gate adoption (replaces uidrive injection) — not started
