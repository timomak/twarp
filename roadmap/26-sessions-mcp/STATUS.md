# Feature 26 — Sessions & Projects MCP: status

Phase: **26c + 26e implemented** (26d in parallel branch)

- 26a spec — PRODUCT.md + TECH.md written 2026-07-31, PR pending review
- 26b read path (`list_sessions`, `get_transcript`, `list_projects` + SessionRegistry) — impl done 2026-07-31 (`app/src/sessions_mcp/`), unit tests green; in-app smoke pending
- 26c events (`watch_session`, `wait_for_completion` + registry broadcast channels) — impl done 2026-07-31 (`app/src/sessions_mcp/events.rs` + registry/bridge), unit tests green; in-app smoke pending
- 26d spawning + projects + external token-gated listener (`create_chat`, `create_project`, settings) — not started
- 26e Codex built-in-server injection parity — impl done 2026-07-31: all three built-ins (`twarp-sessions`, `twarp-browser`, `twarp-computer-control`) additionally served over rmcp streamable HTTP (the transport Codex's `mcp_servers.<name> = { url }` speaks) and merged into the `thread/start`/`thread/resume` config overrides, built-ins winning name collisions; unit tests green; Codex-pane smoke pending
- 26f fleet UX-gate adoption (replaces uidrive injection) — not started
