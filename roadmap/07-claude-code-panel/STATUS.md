# 07 — Claude Code panel

**Phase:** spec-in-review (re-spec PR [#XX](https://github.com/timomak/twarp/pull/XX) open) — **regressed from `impl-in-review`**
**Spec PRs:** [#66](https://github.com/timomak/twarp/pull/66) (PRODUCT.md + TECH.md, merged) · re-spec [#XX](https://github.com/timomak/twarp/pull/XX) (port-and-adapt plan, open)
**Impl PRs:** [#67](https://github.com/timomak/twarp/pull/67) — **abandoned** (rebuilt from primitives instead of porting; see postmortem). Owner to close.

## Scope

Bring back Warp's Agent Mode rendering surface — task list, collapsible thinking blocks, structured tool cards, inline diffs — as a host for **only** the local `claude` CLI (Anthropic's Claude Code) spawned with `--output-format stream-json`. The user's Claude Code login is used implicitly: the `claude` binary handles auth.

Do **not** re-introduce what feature 02 removed at the service layer: no Warp AI accounts, no LLM clients, no billing, no cloud conversation storage. Only the renderer + a claude-code subprocess driver feeding events into it.

Full behavior in [PRODUCT.md](PRODUCT.md); implementation plan in [TECH.md](TECH.md).

## PR #67 postmortem — why this feature regressed

PR #67 bundled 7b–7h into one PR and **rebuilt the panel from GPUI primitives** (`Flex::column()`, `Container::new(...).with_background_color(...)`, `appearance.ui_builder().span(...)`/`.link(...)`) — **zero lines from the deleted Agent Mode rendering layer**, directly against the 7a port-and-adapt mandate. Result: plain-text "tool cards" with no per-tool affordances; a `similar` unified diff rendered as untinted plain spans (no +/- tinting, no hunk headers — none of feature 05's treatment, despite PRODUCT §33); a static `Flex::column` with no `UniformList`/auto-scroll; plain-text assistant output instead of feature 03 markdown (§18); and a `WorkspaceAction::ClaudeCodePanel(...)` dispatch workaround because direct typed-action dispatch "dropped silently" (a focus-wiring symptom, not the fix). See `twarp_07_port_not_rebuild` memory and TECH.md §Postmortem.

**What survives #67 (kept into the next attempt):** the headless **`crates/claude_code` driver crate** (`lib.rs` `Transcript`/`TranscriptEvent`/`TranscriptItem`; `driver.rs` subprocess + defensive JSONL parser + SIGINT interrupt + stdin writer; `sessions.rs` encoded-cwd reader) — **19 passing unit tests, correct and decoupled** — and the **registration scaffolding** (`ToolPanelView::ClaudeCode`, `LeftPanelDisplayedTab`, toolbelt button, render arm, ⌘⌥K via `custom_tag_to_keystroke`, `compute_left_panel_views` push). **Discarded:** the entire primitive panel body and the `WorkspaceAction::ClaudeCodePanel` forwarder.

**Re-spec investigation headline:** the original TECH.md table conflated two distinct deleted surfaces — `ai_assistant/` (the simple **Warp AI Q&A panel**) and `ai/blocklist/` (**Agent Mode**, the tool-card/diff/thinking/todo surface that `warp.dev/agents/claude-code` shows). They were never composed together. The re-spec replaces the table with a per-file decision matrix (TECH.md §Per-file decision matrix): the cleanly reusable primitives are the `inline_action` card chrome (`HeaderConfig`/`RenderableAction`/status icons — already AI-agnostic), the shared markdown stack (`parse_markdown` → `FormattedTextElement`), and feature 05's read-only diff renderer; `code_diff_view.rs` and `requested_command.rs` are the *wrong* port targets (deeply service-coupled) and are rewritten/reused, not ported.

## Sub-phases (re-derived port-shaped — prior 7b–7h ticks cleared)

The previous 7b–7h checklist tracked behavior buckets and was marked done in #67; those ticks are **cleared** because the work shipped against the wrong approach. The new split tracks *which leaf is brought back, what it bridges to, and what stub it needs* (TECH.md §Re-derived sub-phase plan). 7a stays done (the audit/gate); it is amended by this re-spec.

- [x] **7a — Audit + TECH.md (amended by this re-spec).** Gate resolved: **per-component port-and-adapt** from `fea2f7ea`; reparent leaf rendering onto the thin `claude_code::Transcript` model; do **not** `git restore` the service-coupled pieces; rewrite any leaf whose port drags in more `crate::ai::` coupling than rebuilding. Driver in `crates/claude_code`; UI in `app/src/claude_code_panel/`. Re-spec adds the per-file decision matrix, the bridge spec, and the "visually matches Agent Mode" acceptance gate. (Spec [#66], re-spec [#XX].)
- [ ] **7b — Panel shell + ported transcript, stub event source.** Keep registration scaffolding; replace #67's primitive body with the ported transcript renderer (`ai_assistant/transcript.rs::render_message` + the `markdown_parser`→`FormattedTextElement` stack) inside a `UniformList`, fed a **synthetic** `Transcript` (no driver). Dispatch wired the `GlobalSearchView` way; `WorkspaceAction::ClaudeCodePanel` forwarder deleted. Zero + unavailable states. **Acceptance: sample transcript renders in Agent-Mode shape and visually matches `warp.dev/agents/claude-code`.** PRODUCT §1–§7, §16–§20, §60.
- [ ] **7c — Live driver bridge.** Connect the kept `claude_code::driver` to the ported transcript via the per-`TranscriptItem` bridge dispatch; remove the stub source. Streaming/Stop/lifecycle/teardown. PRODUCT §8–§22, §52–§57.
- [ ] **7d — Tool cards.** Port `inline_action_icons` + `inline_action_header` + `requested_action` (co-port `WithContentItemSpacing`); bridge `TranscriptItem::Tool` → `RenderableAction` cards with per-tool summary + generic fallback for unmapped/`mcp__*`. PRODUCT §23–§29.
- [ ] **7e — Diff cards.** Synthesize a unified diff (kept `diff_for_tool`) and render **read-only via feature 05 / `crate::code::inline_diff::InlineDiffView`** (not `code_diff_view.rs` chrome, not plain spans). PRODUCT §30–§33.
- [ ] **7f — Thinking + todos.** Extract the collapsible-thinking helpers (`output.rs`/`common.rs` `render_collapsible_text_block_section` + `format_elapsed_seconds`) for `TranscriptItem::Thinking`; port `todos.rs` bridged to `TodoItem`/`TodoStatus`. PRODUCT §34–§38.
- [ ] **7g — Permissions + input.** Permission-mode selector → `--permission-mode` (robust path first); message `EditorView`; interactive prompts version-gated with §42 degradation. PRODUCT §39–§45.
- [ ] **7h — Session list + resume.** Kept `sessions.rs` reader; resume via `claude --resume <id>`; new-session; zero-state Resume list. No twarp-side session DB. PRODUCT §46–§51.

## Notes

- Closest visual reference is Warp's official Agent Mode UI ([warp.dev/agents/claude-code](https://www.warp.dev/agents/claude-code)). The twarp panel should render the **same shape** — this is the acceptance gate the port-and-adapt approach is built around (the gate #67 failed).
- Each card/diff/thinking block in every impl PR must trace to a **ported leaf** or a **reused master renderer**, never to a fresh `Flex`/`Container`/`Link` tree. Review against TECH.md §Per-file decision matrix.
- **Pin the claude-code version** the driver is tested against; golden stream-json transcripts are `crates/claude_code` parser fixtures (already in the kept crate).
- **Feature flag:** #67 removed `FeatureFlag::ClaudeCodePanel` (panel always-on). 7b decides whether to restore flag-gating for dogfood rollout or stay always-on (acceptable on a personal fork). Record the choice here (TECH.md §Feature flag & rollout).
- Framing matters in STATUS / PR descriptions: feature 02 removed Warp's *AI service*; feature 07 brings back the *rendering layer only*, driven by an external CLI the user already pays for. No LLM connection, no billing, no cloud sync comes back.

## Why this is feature 07 (before rebrand)

The port from upstream's agent crates is the heart of this feature, and the rebrand (now feature 08) renames every `warp_*` / `warpui*` crate. Doing rebrand first would multiply merge effort on every cherry-pick/port. Agent panel must precede rebrand.

## Open decisions (status after re-spec)

1. **Cherry-pick vs `git restore` — RESOLVED.** Per-component port-and-adapt from `fea2f7ea` (TECH.md §Context, §Per-file decision matrix).
2. **Two-surface conflation — RESOLVED (re-spec).** `ai_assistant/` (Q&A) vs `ai/blocklist/` (Agent Mode) are distinct; the decision matrix maps each PRODUCT surface to the real file + verdict (TECH.md §Context).
3. **Tool taxonomy mismatch — RESOLVED.** Generic card for unmapped tools incl. `mcp__*` (PRODUCT §25, TECH 7d). The kept `tool_input_summary` covers `Read`/`Write`/`Edit`/`MultiEdit`/`NotebookEdit`/`Bash`/`BashOutput`/`KillShell`/`Grep`/`Glob`/`WebFetch`/`WebSearch`/`Task`/`TodoWrite`/`ExitPlanMode`.
4. **Multi-session concurrency — DEFERRED to 7h.** One live `claude` process per panel at a time (PRODUCT §9, §49).
5. **Panel placement — DECIDED (provisional): left-panel tab,** resizable; full-pane alternative stays open (PRODUCT §51). Re-decide after 7b renders something real.
6. **Subscription-auth drift — CONCRETE.** Anthropic's **2026-06-15** change routes `claude -p` subscription usage to a separate Agent-SDK credit. Panel meters nothing; surfaces `claude`'s own auth/limit errors verbatim (PRODUCT §55).
7. **Permission control protocol — highest runtime risk.** stdin/stdout permission channel is undocumented; 7g builds `--permission-mode`/`--allowedTools` first, interactive prompts version-gated (TECH §Risks).
8. **Dispatch/focus — RESOLVED (re-spec).** Panel is a self-dispatching `TypedActionView` like `GlobalSearchView` + an `on_left_mouse_down` focus-grab; the `WorkspaceAction::ClaudeCodePanel` forwarder is deleted (TECH.md §The panel).
