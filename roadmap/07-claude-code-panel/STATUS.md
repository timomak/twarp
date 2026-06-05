# 07 — Claude Code panel

**Phase:** impl-in-review (7b PR [#71](https://github.com/timomak/twarp/pull/71) open — main-pane host + `claude` trigger + relocated renderer)
**Spec PRs:** [#66](https://github.com/timomak/twarp/pull/66) (merged) · port-plan re-spec [#68](https://github.com/timomak/twarp/pull/68) (merged) · main-pane re-spec [#70](https://github.com/timomak/twarp/pull/70) (merged)
**Impl PRs:** [#67](https://github.com/timomak/twarp/pull/67) — abandoned (rebuilt from primitives); **owner to close.** · [#69](https://github.com/timomak/twarp/pull/69) — 7b sidebar build, **merged**: landed the placement-agnostic core (`crates/claude_code` + the ported renderer) plus a now-obsolete sidebar host. Rendering was correct; sidebar placement superseded by #70. · **[#71](https://github.com/timomak/twarp/pull/71) — 7b main-pane host (open):** relocated the renderer into `IPaneType::ClaudeCode` (`ClaudeCodeView`/`ClaudeCodePane`), added the `claude`-at-submit terminal trigger, deleted the sidebar host. `cargo check`/`clippy`/`fmt` clean; `claude_code` 19/19.

## Scope

Run `claude` in a terminal → twarp opens a **main-content Claude Code pane** (a tab like an editor/terminal) that drives the local `claude` CLI and renders the session in a polished, Claude-app-style chat UI: streaming markdown, collapsible thinking, structured tool cards, inline diff cards, a task list. A left-sidebar entry lists **past sessions** for the cwd (only when any exist) for reopening. The `claude` binary handles auth.

Do **not** re-introduce what feature 02 removed at the service layer: no Warp AI accounts, no LLM clients, no billing, no cloud conversation storage. Only the renderer + a claude-code subprocess driver feeding events into it.

Full behavior in [PRODUCT.md](PRODUCT.md); implementation plan in [TECH.md](TECH.md).

## History — two missed turns, now corrected

1. **PR #67 (abandoned):** bundled 7b–7h and **rebuilt the panel from GPUI primitives** instead of porting the deleted Agent Mode renderer (plain-text cards, untinted diffs, no `UniformList`, a `WorkspaceAction` dispatch workaround). See `twarp_07_port_not_rebuild` memory.
2. **PR #69 (merged; sidebar placement superseded):** 7b did the port **correctly** — `ai_assistant::transcript::render_message` + the markdown splitter reparented onto `claude_code::Transcript`, themed Markdown, `UniformList`, `GlobalSearchView`-style dispatch, no forwarder — **but rendered it in the left sidebar**, which the owner rejected on sight. The *rendering* was right; the *placement and entry point* were wrong.
3. **This re-spec (#70):** keeps the rendering + driver, moves the chat to a **main-content pane triggered by typing `claude`**, and repurposes the sidebar to a read-only session list. See `twarp_07_ux_direction` memory and TECH.md §Re-spec.

**Kept across all of this (placement-agnostic):** the headless `crates/claude_code` driver crate (`Transcript`/`TranscriptEvent`/`TranscriptItem` + stream-json parser + sessions reader, **19 passing unit tests**) and the ported markdown renderer from #69. Both are now **merged to master via #69** (inside a sidebar host); the re-scoped 7b **relocates** them into the main-content pane rather than re-introducing them from branch `twarp-07b-port`.

## Sub-phases (re-derived for the main-pane host — all ticks cleared)

7a stays done (audit + specs, amended by both re-specs). 7b–7h are re-scoped to land in the pane (TECH.md §Re-derived sub-phase plan).

- [x] **7a — Audit + specs.** Renderer detangling gate resolved (per-leaf port-and-adapt from `fea2f7ea`); main-pane host + terminal trigger audited (`terminal/input.rs` submit hook; `pane_group` `IPaneType`/`CodePane` model). (Specs [#66], [#68], [#70].)
- [x] **7b — Pane host + `claude`-at-submit trigger + ported transcript (stub session).** *(PR [#71](https://github.com/timomak/twarp/pull/71), in review.)* `crates/claude_code` + the ported renderer are **already in master (via #69)** — 7b **relocates** them, it does not re-introduce them. Add `IPaneType::ClaudeCode` + `ClaudeCodePane`/`ClaudeCodeView` (modeled on `CodePane`) and move the merged transcript renderer into the pane. Intercept a top-level `claude` in `terminal/input.rs` → open the pane. Render a synthetic transcript with a docked composer; **delete the #69 sidebar host** (`left_panel` wiring + ⌘⌥K binding). **Acceptance: `claude` opens a real main-content pane whose sample transcript renders as themed Markdown in Claude-app shape, and the sidebar no longer hosts the chat.** PRODUCT §1–§7, §12–§15, §32–§33.
- [ ] **7c — Live driver in the pane.** Wire `claude_code::driver` → pane via the bridge + `apply_event` pump; forward the `claude <prompt>` first turn; streaming/Stop/lifecycle/teardown. PRODUCT §6–§14, §28–§31.
- [ ] **7d — Tool cards.** Port `inline_action` chrome; bridge `Tool` → cards + generic fallback for unmapped/`mcp__*`. PRODUCT §16–§19.
- [ ] **7e — Diff cards.** Synthesize unified diff → render read-only via feature 05 / `InlineDiffView`. PRODUCT §20–§21.
- [ ] **7f — Thinking + todos.** Extract collapsible-thinking helpers; port `todos.rs` bridged to `TodoItem`/`TodoStatus`. PRODUCT §22–§23.
- [ ] **7g — Permissions + composer.** Permission-mode selector → `--permission-mode` (robust path first); interactive prompts version-gated with §26 degradation; composer semantics. PRODUCT §24–§27.
- [ ] **7h — Sidebar session list + resume.** Kept `sessions.rs` reader → read-only left-panel list; resume opens a pane via `--resume`. PRODUCT §35–§38.

## Notes

- **Visual reference: Anthropic's Claude desktop / Claude Code app** (owner-chosen; no Figma). The pane must look like a modern Claude chat UI — themed cards, +/- tinted diffs with hunk headers, collapsible thinking, a real task list, a docked composer. A primitive/plain-text shape fails the gate (PRODUCT §32).
- Every card/diff/thinking block traces to a **ported leaf** or a **reused master renderer**, never a fresh `Flex`/`Container`/`Link` tree (TECH.md §Per-file decision matrix).
- **Trigger must not break real shell commands:** intercept only a bare top-level `claude` token (parsed with the existing completer parser); when in doubt, run it raw (PRODUCT §3, TECH §The trigger).
- **Feature flag — DECIDED: always-on, no flag.** The trigger no-ops when `claude` is absent and the pane is created on demand, so always-on breaks nothing (TECH.md §Feature flag).
- **Pin the claude-code version**; golden stream-json transcripts are `crates/claude_code` parser fixtures (in the kept crate).
- Framing: feature 02 removed Warp's *AI service*; feature 07 brings back the *rendering layer only*, driven by an external CLI the user already pays for. No LLM connection, no billing, no cloud sync comes back.

## Open decisions (after the main-pane re-spec)

1. **Surface placement — DECIDED: main-content pane** (`IPaneType::ClaudeCode`), entered by running `claude`. Sidebar holds the session list only. (Reverses the original "left-panel tab" decision.)
2. **Trigger mechanics — DECIDED: intercept `claude` at terminal submit** and open the pane (PRODUCT §1–§3). Conservative detection is the top correctness risk.
3. **Pane open location — provisional: new tab** in the active tab's group; split/replace alternatives open (PRODUCT §load-bearing-2).
4. **Args forwarding — provisional:** `claude <prompt>` → first turn; bare `claude` → empty composer (PRODUCT §2).
5. **Pane persistence — provisional:** persist at most the session id, restore to a resume affordance; no twarp transcript store (TECH §The pane).
6. **Permission control protocol — highest runtime risk:** `--permission-mode`/`--allowedTools` first; interactive prompts version-gated with §26 degradation (TECH §Risks).
7. **Subscription-auth drift — CONCRETE:** the 2026-06-15 Agent-SDK-credit change; pane meters nothing, surfaces `claude`'s errors verbatim (PRODUCT §30).
8. **Rendering detangle — RESOLVED:** per-leaf port-and-adapt; `code_diff_view.rs`/`requested_command.rs` are the wrong port targets (reuse feature 05 / rewrite from clean chrome). Carried over from the port re-spec.

## Why this is feature 07 (before rebrand)

The renderer port + the agent-crate cherry-picks are the heart of this feature, and the rebrand (feature 08) renames every `warp_*` / `warpui*` crate. Doing rebrand first would multiply merge effort. Claude Code panel must precede rebrand.
