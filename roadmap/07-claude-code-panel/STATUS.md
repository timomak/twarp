# 07 — Claude Code panel

**Phase:** impl-in-review (7b–7h — [#67](https://github.com/timomak/twarp/pull/67) open)
**Spec PR:** [#66](https://github.com/timomak/twarp/pull/66) (PRODUCT.md + TECH.md, merged)
**Impl PRs:** 7b–7h [#67](https://github.com/timomak/twarp/pull/67) (bundled — owner requested all sub-phases in one PR since 7b alone wasn't usable)

## Scope

Bring back Warp's Agent Mode rendering surface — task list, collapsible thinking blocks, structured tool cards, inline diffs — as a host for **only** the local `claude` CLI (Anthropic's Claude Code) spawned with `--output-format stream-json`. The user's Claude Code login is used implicitly: the `claude` binary handles auth.

Do **not** re-introduce what feature 02 removed at the service layer: no Warp AI accounts, no LLM clients, no billing, no cloud conversation storage. Only the renderer + a claude-code subprocess driver feeding events into it.

Full behavior in [PRODUCT.md](PRODUCT.md); implementation plan in [TECH.md](TECH.md).

## Sub-phases

**7a is delivered by the spec PR ([#66]).** The audit (upstream agent crates, feature-02 deletion cross-reference), the detangle-gate decision, the driver-translation layer, and panel placement are all settled in PRODUCT.md / TECH.md. The impl loop begins at **7b**.

- [x] **7a — Audit + TECH.md.** Resolved the gate: **per-component port-and-adapt** from commit `fea2f7ea` (feature-02 spec commit, predates all deletions) — port leaf rendering primitives onto a new thin transcript model; do **not** `git restore` the service-coupled `Requests`/`controller`/`agent_input_footer`; rewrite any component whose port drags in more `crate::ai::` coupling than rebuilding costs. Driver lives in a new headless crate `crates/claude_code`; UI in `app/src/claude_code_panel/`. (Spec PR [#66](https://github.com/timomak/twarp/pull/66).)
- [x] **7b — Resurrect view + event model.** Register `ToolPanelView::ClaudeCode` (+ `LeftPanelDisplayedTab`, toolbelt button, render arm, flag-gated `compute_left_panel_views` push) and the ⌘⌥K `EditableBinding` (chord via `custom_tag_to_keystroke`, **not** `with_key_binding` — feature-06 lesson). Port/adapt the transcript + cards onto a thin `Transcript`/`TranscriptEvent` model. Panel opens and renders the zero state; no claude integration yet. PRODUCT §1–§7, §16–§22 (scaffold). **(Impl PR [#67](https://github.com/timomak/twarp/pull/67).)** New headless crate `crates/claude_code` owns the `TranscriptEvent`/`Transcript` contract (6 unit tests); `FeatureFlag::ClaudeCodePanel` is dogfood-only. ⌘⌥K conflict-free (`cmd-alt-k`/`ctrl-alt-k` unbound); launch-verified no startup panic. The 7b input is a non-editable placeholder. Deferred: live session/streaming/markdown/cwd-header → 7c; rich tool/diff/thinking/todo cards → 7d–7f; editable multi-line input + permissions → 7g; session list/resume + zero-state "Resume…" → 7h.
- [x] **7c — Claude Code subprocess driver.** `crates/claude_code`: spawn `claude -p --input-format stream-json --output-format stream-json --verbose [--resume <id>]`, defensive JSONL parse, emit `TranscriptEvent`s. Assistant text streaming + user-message send + lifecycle/Stop/teardown. PRODUCT §8–§22, §52–§57. **(In [#67].)** Driver lives in `crates/claude_code/src/driver.rs` (9 parser tests); real editable input via `EditorView` (Enter sends, Shift+Enter newline); writer task drains a `async_channel` of user turns into stdin; reader stream feeds `TranscriptEvent`s back via `ctx.spawn_stream_local`; Stop = SIGINT (Unix). Assistant markdown rendering (§18) deferred — current is plain text with soft-wrap.
- [x] **7d — Tool call cards.** Map `tool_use` → tool cards with per-tool summaries; generic card for unmapped/`mcp__*` tools. PRODUCT §23–§29. **(In [#67].)** Per-tool summaries for `Read` / `Write` / `Edit` / `MultiEdit` / `NotebookEdit` / `Bash` / `BashOutput` / `KillShell` / `Grep` / `Glob` / `WebFetch` / `WebSearch` / `Task` / `TodoWrite` / `ExitPlanMode`, generic fallback for any `mcp__*` or unmapped tool, expand/collapse for results longer than 8 lines.
- [x] **7e — Diff rendering.** `Edit`/`MultiEdit`/`Write` → diff cards, reusing feature 05's code-review diff renderer. PRODUCT §30–§33. **(In [#67].)** Diffs are synthesized from `old_string`/`new_string` (or `content` for `Write`) via `similar::TextDiff::unified_diff()` with context radius 3 and rendered inline. The full feature-05 `CodeReviewEditorState` reuse would re-couple to the code-review editor wiring; the +/- line style mirrors the same visual treatment without dragging it in.
- [x] **7f — Todos + thinking.** `TodoWrite` → in-place task list; `thinking` → collapsible "Thought for N seconds" cards. PRODUCT §34–§38. **(In [#67].)** Thinking cards default collapsed, click to expand; `Transcript::apply` updates the live to-do list in place rather than stacking duplicates (a `claude_code` model test guards this). Todos render with pending / in-progress / completed glyphs; completed items strike through.
- [x] **7g — Permissions + input.** Permission-mode selector + `--allowedTools` (robust path first); interactive in-transcript prompts gated behind the pinned-version check with graceful degradation. Multi-line input. PRODUCT §39–§45. **(In [#67].)** Multi-line input via `EditorView` (Enter sends, Shift+Enter newline, empty/whitespace no-op). Permission-mode selector in the header cycles `bypassPermissions` → `acceptEdits` → `plan` → `default` (passes through to `--permission-mode`). Default is `bypassPermissions` so the smoke test doesn't deadlock on the undocumented interactive prompt protocol (TECH §Risks); `Permission` events render as informational cards (the §42 degradation path — no Allow/Deny buttons until the wire protocol is reverse-engineered or an MCP fallback lands).
- [x] **7h — Session list + resume.** List `~/.claude/projects/<encoded-cwd>/*.jsonl`; resume via `claude --resume <id>`; new-session. No twarp-side session DB. PRODUCT §46–§51. **(In [#67].)** `crates/claude_code/src/sessions.rs` lists JSONL files for the encoded cwd, sorts most-recent first, parses first user message best-effort for a title (falls back to "Untitled session"). Zero state shows the resume list inline; clicking spawns with `--resume <id>` (PRODUCT §47, §49 — never two live processes at once). 4 sessions-store tests pass.

## Notes

- Closest visual reference is Warp's official Agent Mode UI ([warp.dev/agents/claude-code](https://www.warp.dev/agents/claude-code)). The twarp panel should render the same shape.
- 7d–7h are independent enough that each can ship as its own impl PR; the panel is usable but visually plain after 7c.
- Likely scope-cut candidates: 7f and 7h. 7h is only worth shipping if claude-code's session-store schema is stable enough to read directly (TECH.md keeps the on-disk title parse best-effort).
- **Pin the claude-code version that 7c is tested against** — that's where upstream-protocol drift will surface. Capture golden stream-json transcripts as `crates/claude_code` parser fixtures.
- Gate the whole panel behind `FeatureFlag::ClaudeCodePanel`; ship 7b–7h dogfood-only, promote via `promote-feature` once the driver is proven.
- Framing matters in STATUS / PR descriptions: feature 02 removed Warp's *AI service*; feature 07 brings back the *rendering layer only*, driven by an external CLI the user already pays for. No LLM connection, no billing, no cloud sync comes back.

## Why this is feature 07 (before rebrand)

The port from upstream's agent crates is the heart of this feature, and the rebrand (now feature 08) renames every `warp_*` / `warpui*` crate. Doing rebrand first would multiply merge effort on every cherry-pick/port. Agent panel must precede rebrand.

## Open decisions (status after spec)

1. **Cherry-pick vs `git restore` — RESOLVED.** Per-component port-and-adapt from `fea2f7ea`; reparent leaf rendering onto a new thin model, drop service-coupled pieces (TECH.md §Context, §Proposed changes).
2. **Tool taxonomy mismatch — RESOLVED.** Generic card for unmapped tools, including `mcp__*` (PRODUCT §25, TECH 7d). STATUS's old tool list extended with `MultiEdit`, `BashOutput`, `KillShell`, `NotebookEdit`, `ExitPlanMode`.
3. **Multi-session concurrency — DEFERRED to 7h.** Invariant fixed now: one live `claude` process per panel at a time (PRODUCT §9, §49). In-panel sub-tabs are a follow-up.
4. **Panel placement — DECIDED (provisional): left-panel tab,** resizable. Flagged as the most likely respec point (cf. feature 05); full-pane alternative stays open (PRODUCT §51, TECH §Risks). Decide for real after 7b renders something.
5. **Subscription-auth drift — CONCRETE.** Anthropic's **2026-06-15** change routes `claude -p` subscription usage to a separate Agent-SDK credit. The panel meters nothing; it surfaces `claude`'s own auth/limit errors verbatim (PRODUCT §55). Call it out in dogfood release notes.
6. **Permission control protocol (new, highest risk).** The stdin/stdout permission channel is undocumented/unstable; 7g builds the official `--permission-mode`/`--allowedTools` path first and treats interactive prompts as a version-gated enhancement (TECH §Risks).
