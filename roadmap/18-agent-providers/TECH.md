# Multi-provider agent pane (Codex backend) — TECH

Companion to [PRODUCT.md](PRODUCT.md); §N references its invariants. Grounded in the coupling map verified 2026-07-16 (paths/lines checked against the tree) and protocol research against codex-cli 0.135.x-era docs + the openai/codex source.

## The seam (why this is an extraction, not a rewrite)

The pane already has a normalized boundary: **`claude_code::TranscriptEvent`** (`crates/claude_code/src/lib.rs:170`) — documented as carrying no claude wire shapes — feeding `Transcript`/`TranscriptItem` (pure view model) and the 10k-line `ClaudeCodeView`, which is provider-neutral except for ~10 call sites. All claude-specific knowledge is concentrated in:

- `crates/claude_code/src/driver.rs` (~2.1k) — `spawn_session` arg contract (:135-184), stream-json `Parser` (:509-1010), `send_user_message`/`send_control_response` (:301-370), `send_interrupt` control_request (:231-248), usage parsing (:1249+).
- `crates/claude_code/src/sessions.rs` — `~/.claude/projects/<enc-cwd>/*.jsonl` store layout (:37-62), list/history/fork.
- `crates/claude_code/src/launch.rs` — claude flag names (:78-106).
- App side: `begin_session` calling `spawn_session` directly (`claude_code_view.rs:3173`), claude-JSON permission answers (:3318-3353, :2884-2899), model discovery (`claude_code_models.rs:55-93`), trigger `CLAUDE_PROGRAM` (`terminal/input.rs:7355`), snapshot without provider (`app_state.rs:1086-1099`).

Feature 16 already ships the **metadata** adapter: `CLIAgent{Claude,Codex,Gemini}` + `CLIAgentAdapter` (`app_state.rs:551/727` — executable/spawn-spec/login-probe/capabilities/models/efforts/permission-modes), consumed by a fully capability-driven settings page. 18 adds the **runtime** face and wires the view through it.

## 18a — `AgentDriver` extraction (zero behavior change; §28–§29)

New trait (in `crates/claude_code`, which becomes the shared driver crate — **no crate rename this feature**):

```rust
trait AgentDriver: Send + Sync {
    fn spawn(&self, opts: SpawnOptions) -> Result<SpawnedSession>;      // resume via opts.resume
    fn send_user_turn(&self, stdin: &mut ChildStdin, msg: OutgoingMessage) -> Result<()>;
    fn interrupt(&self, stdin: &mut ChildStdin, state: &TurnState) -> Result<()>;
    fn answer(&self, stdin: &mut ChildStdin, req: &PendingRequest, decision: Decision) -> Result<()>;
    fn parse_line(&mut self, line: &str) -> Vec<TranscriptEvent>;       // per-provider translator
    fn sessions(&self) -> &dyn SessionStore;                             // list/load_history/fork
    fn capabilities(&self) -> DriverCapabilities;                        // fork/steer/cost/thinking/...
}
```

- `Decision` is a typed enum (AllowOnce / AllowAlways / Deny / Answer(value)) replacing the raw `{"behavior":"allow"}` serde_json at `claude_code_view.rs:3347` / `:2889` — the claude driver serializes it to today's exact JSON (golden tests prove byte-compat).
- `ClaudeDriver` = today's free functions moved behind the trait; `event_stream` (driver.rs:417) and `SpawnedSession{child,stdin,events}` stay shared plumbing.
- `begin_session` (view:3173) resolves the driver from the pane's `CLIAgent`; `on_session_spawned` (:3208) — drain loop, `StdinCommand{Turn,Control,Interrupt}`, epoch guard — is already neutral and stays verbatim.
- **Persistence**: `ClaudeCodePaneSnapshot` (+ `claude_code_panes` table) gains `provider TEXT NOT NULL DEFAULT 'claude'` (diesel migration; absent ⇒ Claude — §29). Sidebar session rows carry provider.
- **Golden-transcript tests**: recorded stream-json fixtures replayed through `ClaudeDriver::parse_line` must produce the identical `TranscriptItem` sequence as pre-refactor (snapshot test checked in before the move, re-run after).

## 18b — `CodexDriver` via `codex app-server` v2 (§5–§10)

**Why app-server, not `exec --json`**: only app-server streams deltas (`item/agentMessage/delta`, `item/commandExecution/outputDelta`), carries interactive approvals as server→client requests, supports `turn/interrupt`/`turn/steer`, and does in-process `thread/resume` — exec-json is snapshot-only with no approval round-trip. It is the API OpenAI's own IDE/desktop surfaces use.

- **Process model**: one `codex app-server` child per pane (decision 6). Wire = JSONL JSON-RPC-ish over stdio (no `"jsonrpc"` field — do not send it); mandatory handshake `initialize` → `initialized` notification before anything else.
- **Protocol types**: vendored into `crates/claude_code/src/codex/protocol.rs` — hand-rolled serde structs for exactly the subset we consume, validated against `codex app-server generate-json-schema` output for the **pinned minimum version** (checked in under `fixtures/`). No git-dependency on the codex workspace (their protocol crates aren't published; the crates.io `codex-protocol` is an unrelated third-party fork — do not use).
- **Lifecycle mapping**: `thread/start {model, cwd, approvalPolicy, sandbox}` → `TranscriptEvent::Init(thread.id)`; `thread/resume {threadId}` for restore (never parse rollout files — explicitly unstable). Persist threadId in the session_id slot.
- **Event mapping** (parse_line):

| codex v2 | TranscriptEvent |
|---|---|
| `item/started(agentMessage)` + `agentMessage/delta` + `item/completed` | assistant text delta / final (completed item is authoritative) |
| `reasoning` item + `summaryTextDelta` | thinking (collapsed) |
| `commandExecution` started / `outputDelta` / completed {aggregatedOutput, exitCode} | ToolCall(command) → live output → ToolResult |
| `fileChange` item {changes[], status} (+`turn/diff/updated`) | edit tool card + diff card |
| `mcpToolCall` / `webSearch` / `plan` (`turn/plan/updated`) | MCP card / web card / todos |
| `turn/completed {usage}` / `turn/failed {error}` | Ended + Usage (tokens only — no cost field exists; §21) |
| unknown item kind | generic card (§6) |

- **Interrupt** (§7): `turn/interrupt {threadId, turnId}` — track current turnId from `turn/started` in `TurnState`. Same lesson as claude: never SIGINT as primary stop.
- **Approvals**: `item/commandExecution/requestApproval` and `item/fileChange/requestApproval` are JSON-RPC **requests with ids — the turn blocks until the client responds**. Surface as `TranscriptEvent::PermissionRequest`; `answer()` maps Decision → `accept | acceptForSession | decline` (`cancel` = deny-and-abort wired to Deny+Stop). **Guaranteed-reply invariant** (§15): every PendingRequest is answered on all exits — pane close, session end, user timeout — mirroring the AskUserQuestion-wedge fix.
- **Feature flag**: `CodexAgentBackend` (twarp_features), default off → dogfood-on in 18d.
- **Fixture tests**: recorded app-server transcripts under `fixtures/codex/` replayed through `CodexDriver::parse_line` (same harness as claude goldens).

## 18c — Access pill + unified approvals (§11–§17)

Mapping (pill stop → provider-native):

| Stop | Claude (`--permission-mode`) | Codex (`thread/start` sandbox + approvalPolicy) |
|---|---|---|
| Read-only | `plan` | `read-only` + `on-request` |
| Ask to edit | `default` | `workspace-write` + `untrusted` |
| Edits allowed | `acceptEdits` | `workspace-write` + `on-request` |
| Full access | `bypassPermissions` | `danger-full-access` + `never` |

- Pill = the existing permission-mode pill generalized; popover lists native names (§12); non-canonical provider configs (set via flags/config) display native-only (§12). Mid-session change: Claude keeps today's mechanism; Codex applies on next `turn/start` (per-turn overrides).
- Approval card: one component consuming `PermissionRequest` regardless of provider; keyboard handling unchanged. `claude_alias_launch_options`-equivalent parsing marks bypass flags → Full access (§16).

## 18d — entry points, settings light-up, sessions (§1–§4, §18–§20, §23–§27, §31–§32)

- **Trigger**: generalize `CLAUDE_PROGRAM` (`input.rs:7355`) to a command→`CLIAgent` table (reuse `matched_agent()` in `settings/ai.rs:1989`); alias expansion identical to claude's (session.alias_value); unsupported flags → raw CLI fallthrough (§2). `parse_launch_args` grows a codex flag dialect in `launch.rs`.
- **Settings**: implement `CLIAgentAdapter` for Codex (`executable_name="codex"`, install probe = PATH, login probe = cheap read-only auth check, e.g. `codex login status`; models/efforts fetched via a short-lived `app-server` `model/list` call and cached) → `CLIAgent::adapter()` returns it; `capabilities().enabled = true`. Page needs zero edits (16d acceptance).
- **Auth flow** (§18): logged-out card's action opens a terminal split running `codex login` (device-code variant when headless); completion detected by re-probe (and/or `account/loginCompleted` when a server is already up).
- **Sessions sidebar** (§31–§32): `SessionStore` impl for codex reading its local session index for the cwd; provider glyph + filter chips in `left_panel.rs` ClaudeSessions view (rename user-visible strings to "Agent sessions").
- **Min-version pin** (§20): `codex --version` parse at spawn; below-min → upgrade card, no protocol attempt.

## 18e — capability polish (§9, §21–§22, §25)

- Fork: codex `thread/fork` behind `capabilities().fork`; turn-count parity tests mirroring the 7-era fork fixes (§9).
- Usage line: provider-shaped (Claude: cost+tokens as today; Codex: tokens + `account/rateLimits` quota when ChatGPT-authed) (§21).
- Steering (`turn/steer`) behind `capabilities().steer` — composer stays enabled mid-turn for codex, queued send for claude (§25). Ship dark if flaky; capability-gated so it's droppable.
- Error taxonomy: map `turn/failed.error` codes (contextWindowExceeded, usageLimitExceeded, auth) to readable ended states (§22).

## Risks

1. **Protocol drift** — codex releases fast. Mitigation: pinned min version + vendored types + schema-diff check in fixtures; upgrade card rather than best-effort parsing (§20/§34).
2. **Unanswered approval wedge** (the 2h AskUserQuestion freeze, now with JSON-RPC ids): the guaranteed-reply invariant is enforced in one place (PendingRequest registry with drop-guard replying `decline` on teardown).
3. **View churn vs feature 19**: 18 lands after 19 by owner sequencing; 18 makes **no layout/style changes** (PRODUCT non-goal) so 19's restyle isn't churned.
4. **Two-process resource creep**: one app-server per pane is fine at pane counts twarp sees; revisit shared-process if profiling says otherwise (decision 6).
5. **Claude regressions from the extraction**: golden transcripts + the 07-era tripwires (Stop-not-SIGINT, FocusSelf, SelectableArea, persistence checklist) re-verified in 18a's PR.

## Validation

- 18a: golden-transcript snapshot equality (claude fixtures) + full manual claude smoke (PRODUCT 18a steps) + existing pane tests green.
- 18b: codex fixture replay tests; live smoke per PRODUCT 18b; kill -9 the app-server child mid-turn → §10 ended-state.
- 18c: approval matrix table test (4 stops × 2 providers × allow/always/deny); §15 wedge test (deny + pane-close during pending approval).
- 18d: probe matrix (not installed / installed+logged-out / logged-in); settings seeding precedence test (launch flag → Chat row → fallback); mixed side-by-side panes (§30).
- Fleet gates: functional verify `cargo build --bin twarp-oss` minimum + per-PR targeted `cargo test -p claude_code`; UX-drive gate against PRODUCT `## Smoke test`; opposite-model staff review.
