# 07 — Claude Code panel

**Phase:** not-started
**Spec PR:** —
**Impl PRs:** —

## Scope

Bring back Warp's Agent Mode rendering surface — task list, collapsible thinking blocks, structured tool cards, inline diffs — as a host for **only** the local `claude` CLI (Anthropic's Claude Code) spawned with `--output-format stream-json`. The user's Claude Max subscription is used implicitly: the `claude` binary handles auth.

Do **not** re-introduce what feature 02 removed at the service layer: no Warp AI accounts, no LLM clients, no billing, no cloud conversation storage. Only the renderer + a claude-code subprocess driver feeding events into it.

## Sub-phases

- [ ] **7a — Audit + TECH.md.** Map upstream Warp's agent panel crates (view, conversation-event model, LLM-client layer). Cross-reference feature 02's deletion list. Confirm the rendering layer can be detangled from the LLM-client layer. Output: `roadmap/07-claude-code-panel/TECH.md` describing cherry-pick targets and the driver-translation layer. **Gate**: if layers can't be detangled, pivot to a clean-room ACP-based build.
- [ ] **7b — Resurrect view + event model.** Cherry-pick (or `git restore` + modernize) the agent-panel view and conversation-event types from upstream. Stub the backend. The panel registers, opens, and renders an empty conversation. No claude integration yet.
- [ ] **7c — Claude Code subprocess driver.** New crate. Spawns `claude --output-format stream-json [--resume <id>]`, parses JSONL, emits events into the conversation model. Initially: assistant text streaming + user messages only.
- [ ] **7d — Tool call cards.** Map claude-code's `tool_use` events (`Read`, `Edit`, `Write`, `Bash`, `Grep`, `Glob`, `WebFetch`, `WebSearch`, `Task`, `TodoWrite`, etc.) to the resurrected tool-card UI. Unmapped tools render as a generic card.
- [ ] **7e — Diff rendering.** Map `Edit` / `Write` tool events into the diff-card UI (path / old / new). Reuse rendering primitives from feature 05's Open Changes panel where it makes sense.
- [ ] **7f — Todos + thinking.** Claude Code's `TodoWrite` tool → task-list panel. `thinking` blocks → collapsible "Thought for N seconds" cards.
- [ ] **7g — Permissions + input.** Permission-request events → in-panel confirmation UI. Input box writes to the claude subprocess stdin. Reuse Ctrl+G rich-input editor if feasible.
- [ ] **7h — Session list + resume.** UI for `~/.claude/projects/*` sessions, with a new-session button. Resume reads claude's existing session store — no twarp-side session DB.

## Notes

- Closest visual reference is Warp's official Agent Mode UI ([warp.dev/agents/claude-code](https://www.warp.dev/agents/claude-code)). The twarp panel should render the same shape.
- 7d–7h are independent enough that each can ship as its own impl PR; the panel is usable but visually plain after 7c.
- Likely scope-cut candidates: 7f and 7h. 7h is only worth shipping if claude-code's session-store schema is stable enough to read directly.
- Pin the claude-code version that 7c is tested against — that's where upstream-protocol drift will surface.
- Framing matters in STATUS / PR descriptions: feature 02 removed Warp's *AI service*; feature 07 brings back the *rendering layer only*, driven by an external CLI the user already pays for. No LLM connection, no billing, no cloud sync comes back.

## Why this is feature 07 (before rebrand)

Cherry-picks from upstream's agent crates are the heart of this feature, and the rebrand (now feature 08) renames every `warp_*` / `warpui*` crate. Doing rebrand first would multiply merge effort on every cherry-pick. Agent panel must precede rebrand.

## Open decisions

1. **Cherry-pick vs `git restore`.** 7a decides. Restoring from feature 02's pre-removal commits might be more stable than tracking upstream head.
2. **Tool taxonomy mismatch.** Warp Agent Mode's tool set ≠ claude-code's. Some claude tools (`Grep`, `WebFetch`, `TodoWrite`) have no Warp equivalent; some Warp tools have no claude equivalent. Default: generic card for unmapped tools.
3. **Multi-session concurrency.** One panel = one claude session, or one panel hosts multiple sessions (tabs within the panel)? 7h decides.
4. **Panel placement.** Left-panel tab (like 04 Shortcuts and 05 Open Changes), right side, or full-tab overlay (like upstream Warp's "Esc for terminal" mode)? 7a should propose.
5. **Subscription-auth drift.** If Anthropic later restricts Max-subscription usage of `--output-format stream-json` to in-IDE-only contexts, the panel breaks. Worth tracking; not blocking.
