# Feature 26 — Sessions & Projects MCP: technical spec

Behavior: see `PRODUCT.md` (invariants referenced as P#N below).

## Context

twarp has two built-in MCP servers with an established shape this feature clones:

- `app/src/browser_mcp.rs` — the reference. `BrowserMcpRuntime::start` (255–269) builds a dedicated 1-worker tokio runtime + root `CancellationToken`; `start_server` (293–350) binds an rmcp `SseServer` on `127.0.0.1:0` (`/sse` + `/message`), one service instance per SSE connection via `with_service`. `session_server_url` (272–289) lazily creates one listener per Claude session, stamping `scope_session_id` on each service.
- `app/src/computer_control/mcp.rs` — same shape (694–783); also the reference for main-thread↔runtime handoff and for the pane-header indicator/kill-switch UI that provenance badges (P#22) will mirror.
- Injection: `claude_mcp_config_json` (`app/src/claude_code_view.rs:12376-12414`) merges registry + built-in server configs → `--mcp-config` (`crates/claude_code/src/driver.rs:245`). **Codex gap**: `codex_config_overrides` (`claude_code_view.rs:3724-3728`) passes registry-only config to `thread/start` (`crates/claude_code/src/codex/mod.rs:225,233`); built-ins are never injected (P#20).
- Pane creation: `open_claude_code_tab` (`app/src/workspace/view.rs:13153-13182`) = `add_terminal_tab` + `open_claude_code_pane` (13385); `create_project_chat_in_directory` (19822) already opens a chat inside a project.
- Sessions stores: `crates/claude_code/src/sessions.rs` (`StoredSession`: id/title/timestamp/jsonl_path/provider; `list_sessions_for`, `load_history`) and `codex/sessions.rs` (reads `session_meta` head for id+cwd, mtime cache).
- Status: computed in `ClaudeCodeView` — `ConversationStatus` (`claude_code_view.rs:4540-4577`), attention transitions (3925–4018), deferred completion (`maybe_fire_deferred_completion` 4598+), background work (`has_active_background_work` 4582). This is the single source `list_sessions` status must share (P#4, P#11).
- Message submission: `submit_message` (`claude_code_view.rs:3130`) / `send_user_message` driver path — `create_chat`'s prompt goes through this, not PTY bytes.
- Projects: `ProjectManagementModel` (`app/src/projects.rs:28`), `create_project(NewProjectSource, …)` (`view.rs:19674`).

## Proposed changes

New module `app/src/sessions_mcp.rs` + `app/src/sessions_mcp/` submodules, cloned from the browser bridge shape: `SessionsMcpBridge` (main-thread handle), `SessionsMcpRuntime` (tokio runtime + listeners), `SessionsMcpServer` (per-connection rmcp service, `#[tool_router]`).

**Registry facade (26b).** The MCP runtime cannot touch views directly. Add a main-thread `SessionRegistry` model owned by app state that mirrors, per live session: id, provider, cwd, title, project, status, depth, origin. `ClaudeCodeView` publishes into it at the same points it repaints the tab (status change, title change, close) — one new `update_registry(…)` call next to `TabStatusChanged` emission, guaranteeing P#4 by construction. Extract the `ConversationStatus` computation (4540) into a pure `fn conversation_status(inputs) -> ConversationStatus` shared by renderer and registry. The registry keeps a `tokio::sync::watch`/`broadcast` pair the MCP runtime reads without main-thread hops for `list_sessions` snapshots.

**Transcript access (26b).** `get_transcript` (P#6–8): the registry also holds, per live session, an `Arc`-shared append-only `Vec<TranscriptItem>` (index, role, text) that `ClaudeCodeView` appends to as it applies `TranscriptEvent`s — a deliberately flat projection, not the render model, so the seam survives feature-18 churn. Past sessions: map `load_history` / Codex rollout read into the same shape on demand. `list_projects` reads `ProjectManagementModel` via a main-thread request-response channel (small, rare calls).

**Events (26c).** Per-session `broadcast::Sender<SessionEvent>` (`Item(TranscriptItem)`, `Status(ConversationStatus)`, `Closed`) in the registry, fed by the same publish points. `watch_session` forwards onto the SSE connection as MCP notifications; `wait_for_completion` is a `tokio::select!` over the receiver + timeout, with the deferred-completion rule already encoded because the registry only flips to `done_*` when the view does (P#11–12).

**Spawning + projects (26d).** `create_chat` sends a `SpawnRequest` over a channel to the main thread; a workspace-level handler validates (cwd exists, project exists, cap, depth — P#15, 23–24, atomically against the registry so races can't exceed the cap, P#28), then runs the `open_claude_code_tab` recipe with a `SpawnOrigin { parent: SessionOrigin, depth }` attached to the pane, and submits the prompt via `submit_message`. Origin persists with the pane snapshot (extend the 7m persistence payload) and renders as a header chip next to the computer-control indicator (P#22). `create_project` calls `create_project(NewProjectSource::ExistingFolder…)` on the main thread and returns the model's row.

**External listener (26d).** Second listener in the same runtime: fixed configurable port, axum middleware checking `Authorization: Bearer` against a token file (`0600`, under the app-support dir; generated on enable, path surfaced in settings). New settings-page toggle + regenerate button + port field; disable cancels the listener's child token (severs watchers, P#21, 12, 30). Unauthorized/surface-disabled map to structured MCP errors (P#27).

**Codex parity (26e).** Extend `codex_config_overrides` to merge the three built-in servers' SSE URLs into the `thread/start`/`thread/resume` config the same way the registry map is built (`mcp_registry.rs:466`), gated on Codex's SSE-transport support (verify against the pinned app-server protocol; if Codex only accepts stdio MCP specs, add a thin stdio→SSE proxy subcommand to the existing CLI binary and spec that in this phase's PR). Codex views publish into the registry identically — status inputs come from the shared `TranscriptEvent` seam so no per-provider divergence (P#7, 20).

**26f fleet adoption** lives in the fleet scripts (other-mac + this repo's `fleet/` tooling): swap the UX-gate driver's uidrive injection for token-auth SSE calls; keep computer-control screenshots.

## Testing and validation

- Unit (`app/src/sessions_mcp/`, run via nextest on CI — local presubmit is unreliable on this machine): status projection parity (P#3–4) table-tests over `conversation_status` inputs; transcript index stability under concurrent append/read (P#8); cap/depth race tests with N racing spawn requests (P#23–24, 28); token middleware accept/reject/regenerate (P#21).
- Store mapping tests: Claude jsonl and Codex rollout fixtures → identical `TranscriptItem` shape (P#7).
- Manual smoke per phase (launch via `./script/run`, verify in-app — the keybinding/UI rule applies):
  - 26b: from a Claude pane, ask the agent to call `list_sessions`/`get_transcript`/`list_projects`; cross-check against sidebar and tab states.
  - 26c: `wait_for_completion` from pane A on pane B while B runs a long turn with a background script — checkmark and tool return must coincide (P#11); close B mid-wait → `closed` (P#12).
  - 26d: in-pane `create_chat` (badge, cap error at 5th, depth error at 3), external `curl -H "Authorization: Bearer …"` create+wait round-trip, bad token rejected, toggle-off severs a live watch; `create_project` appears in sidebar.
  - 26e: repeat the 26b/26d smokes from a Codex pane.
  - 26f acceptance: one full fleet UX-gate round over the new surface (P: Fleet adoption section).

## Parallelization

Mostly sequential — 26c/26d/26e all build on 26b's registry, and each phase is one reviewable PR per the repo's flow. Within 26d, the external listener + settings UI is separable from spawn/projects; if wall-clock matters, run two worktree agents (`.claude/worktrees/26d-spawn` on `feat/26d-spawn`, `.claude/worktrees/26d-external` on `feat/26d-external-listener`), owner-per-file: spawn agent owns `workspace/view.rs` + pane persistence; listener agent owns `sessions_mcp/external.rs` + `settings_view`. Merge spawn first; listener rebases. 26e can start in parallel with 26c (touches only the Codex config path + a proxy, not the registry internals).

## Risks and mitigations

- **Codex MCP transport unknown** (SSE vs stdio-only): resolved at the top of 26e before wiring; the stdio→SSE proxy fallback is scoped above so it can't stall the phase.
- **Main-thread publish points drift**: status parity is enforced by sharing one pure function and publishing at the repaint sites; a debug assertion comparing registry status to rendered status in dogfood builds catches drift early.
- **External surface = local RCE-adjacent**: mitigations are the token file perms, fixed localhost bind, off-by-default setting, cap/depth, and visible provenance. No auth on the in-app ephemeral-port servers is pre-existing and unchanged here.
- **Feature 18 transcript churn**: the flat `TranscriptItem` projection is the compatibility layer; only the projection function may need updating.
