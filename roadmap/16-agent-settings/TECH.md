# Agent settings — TECH

Companion to [PRODUCT.md](PRODUCT.md). Section numbers in the Testing table refer to PRODUCT.md invariants.

> **File:line references are point-in-time** (gathered 2026-07-01). The `warp` app crate churns; re-grep the symbols named below before editing rather than trusting a line number.

## Context

Feature 07 left agent configuration in three places:

- **Composer pills** — runtime per-pane cycling of model/effort/permission mode (7g/7m), `app/src/claude_code_view.rs`.
- **Per-session defaults** — `ClaudeSessionDefaultsModel` (`app/src/claude_code_session_defaults.rs`) persisting *last-used* `{model, effort, permission_mode}` to SQLite `claude_session_defaults` (`app/src/persistence/sqlite.rs:3142, 3168`).
- **Hardcoded fallbacks** — in `ClaudeCodeView::new()` (`claude_code_view.rs:~834`), today's precedence is `LaunchOptions → persisted last-used → hardcoded`.

This feature adds an app-level settings surface, provider abstraction, keychain-backed secrets, and — because PRODUCT scope includes them — the two suggestion **consumers** (reply ghost-text, terminal AI-suggestion) that read the new config.

## What already exists (reuse, don't rebuild)

- **Settings page framework.** Pages register via `SettingsSection` (`app/src/settings_view/mod.rs:174`) + `SettingsPageViewHandle` (`app/src/settings_view/settings_page.rs:93`); each is a view struct + `…PageAction` + `…PageEvent` (reference `app/src/settings_view/features_page.rs`). Instantiated in `SettingsView::new()` (`mod.rs:~944`), pushed into `settings_pages` (`~1085`) + `nav_items` (`~1113`), dispatched in `child_view()` (`~114`). See the **"Agents umbrella … removed from the sidebar"** comment (`~1109`) — precedent for re-adding an agent page.
- **Settings persistence macro.** Non-secret groups use `define_settings_group!` (TOML). App-level blobs live in `PersistedData` (`app/src/persistence/mod.rs:199`) over diesel/SQLite.
- **Provider enum (skeleton).** `CLIAgent` (`app/src/app_state.rs:530`): `Claude | Codex | Gemini | Unknown` with `from_serialized_name` / `serialized_name` / `display_name` (`~546`) — **all impls are dead stubs**. This is the backend-selector model; flesh out capabilities, don't invent a new enum.
- **Claude spawn seam.** `LaunchOptions` (`crates/claude_code/src/launch.rs:22`) → view fields (`claude_code_view.rs:~923`) → `spawn_options()` (`~2514`) → `SpawnOptions{model, effort, permission_mode, …}`. Single point the §14–§19 Chat-row config must feed.
- **Ghost-text infra (reply generator target).** `EditorView::set_autosuggestion` (`app/src/editor/view/mod.rs:~3451`), `AutosuggestionType::AgentModeQuery`, accept keybinding `editor_view:insert_autosuggestion` (Tab/→), soft-wrap/height handling — all already built. The Claude composer *is* this editor (`claude_code_view.rs` calls `set_placeholder_text`). 16e feeds a string in; it does not build ghost text.
- **Terminal suggestion infra (terminal generator target).** `Autosuggester` trait (`app/src/terminal/input.rs:~2805`) with async `on_autosuggestion_result`, `set_autosuggestion` (`~7767`), `AutosuggestionType::Command{was_intelligent_autosuggestion}`, `is_intelligent_autosuggestions_enabled`, accept/ignore tracking. The AI layer historically pointed at Warp's cloud backend (gone). 16f supplies a new backend behind this trait; instant history suggestions stay untouched (§41).

## What's net-new

- **App-level agent settings model** (a `define_settings_group!` group and/or a `PersistedData` field for the nested matrix): `backend: CLIAgent`, `actions: {chat, terminal_suggest, reply_suggest} → {provider: CLIAgent|Inherit, model, effort}`, `chat.permission_mode`, and `enable_reply_suggestions` / `enable_terminal_suggestions` toggles.
- **The Agent settings page** view + registration.
- **A keychain seam.** **No existing secret storage in twarp** — the single biggest new dependency. Add an OS-keychain wrapper (macOS Keychain; **grep first** for `security-framework` / `keyring` / `objc2` Keychain usage before adding a dep). Keys stored/looked-up by stable service+account per `CLIAgent`; settings TOML stores a *presence* flag only.
- **A capability model on `CLIAgent`** — `supports_permission_modes()`, `supports_effort()`, `models()`, `install_probe()`, `login_probe()` — driving §5 capability-aware UI + §6–§13 auth row.
- **A `SuggestionProvider` abstraction** — given `{context, provider, model, effort}` returns `Option<String>` off-thread. Two impls: **resident-CLI** (Claude subscription → a persistent/`-p` `claude` invocation, reusing the driver's spawn plumbing; heavy, debounced) and **API-key HTTP** (fast completion; low-latency). Shared by 16e and 16f; the provider/auth per call comes from the matrix row.

## Sub-phase plan

Config lands first (immediately useful for chat); generators last (they depend on it).

### 16a — Agent page scaffold + unified authoritative Chat config
Register the **Agent** page (`SettingsSection` + `SettingsPageViewHandle` + `AgentSettingsPageView`/`…Action`/`…Event` per `features_page.rs`; wire `SettingsView::new()` + both vectors + `child_view()`). Add the settings group with `backend` (Claude enabled; Codex/Gemini disabled entries, §2/§4) and the `actions.chat` row `{provider, model, effort, permission_mode}`. Render the **Chat & history** row as the single source of truth (no separate defaults block — decision "unify"). Feed it into the spawn seam: change `ClaudeCodeView::new()` precedence to **launch flag → Chat-row config → hardcoded** (§16), and make it **authoritative** — new panes start at the Chat row and **do not** consult `claude_session_defaults` last-used for the starting value (that table is now only for pane *restore*, if kept). **Acceptance:** the Agent page exists; changing the Chat row's model/effort/mode changes what a freshly-typed `claude` pane starts with, overriding whatever the pills were last cycled to; pills still override that pane at runtime; open panes unaffected. PRODUCT §1–§5, §14–§19, §23–§24.

### 16b — Auth status + API key in the OS keychain
Add the keychain wrapper + per-backend **auth status** row (§6–§13). Implement `CLIAgent::install_probe()` (PATH) and `login_probe()` (cheap, side-effect-free, off-thread; for Claude a non-interactive status probe that starts no turn — **empirically verify the exact command against the pinned `claude` CLI** per the feature-07 rule). API-key field → keychain keyed by `CLIAgent`; settings store presence only; masked field with Replace/Remove (§13); status resolves login-vs-key-vs-none and updates on save/clear (§12). **Acceptance:** saving a key persists in Keychain (not the TOML), the status row is correct, clearing with a logged-in CLI falls back to local login, no secret in settings files or telemetry. PRODUCT §6–§13, §25–§26.

### 16c — Per-action matrix (terminal + reply rows)
Extend the group with `actions.terminal_suggest` and `actions.reply_suggest` (`{provider: CLIAgent|Inherit, model, effort}`, independent per action, §14/§20). Render the **Models by action** rows, capability-aware per row's provider (§5, §21), with **"Default" = inherit Chat backend** (§20) and per-row auth warnings (§21–§22). Add the two enable toggles (§30, §40), off by default. Persist/validate; corrupt → all-"Default", generators off (§24). **Acceptance:** each row persists independent provider/model/effort; toggles persist; auth warnings surface; no generator behavior yet. PRODUCT §14, §20–§24, §30, §40.

### 16d — Provider abstraction hardening
Flesh out the `CLIAgent` capability model + define the **adapter seam** future Codex/Gemini drivers implement (a trait normalizing spawn + auth; Claude is the only impl). Codex/Gemini stay disabled selector entries. **Acceptance:** the page renders purely from `CLIAgent` capabilities (Claude → Ask/Plan/Bypass + effort; a capability-less test backend hides them); adding a backend = implementing the adapter + flipping `enabled`, not editing the page. PRODUCT §2, §4, §5.

### 16e — Chat reply suggestions (ghost text)
Build the reply generator on the existing editor ghost-text infra. On a Claude turn `Ended` with an empty, focused, idle composer (§31, §36), call `SuggestionProvider` with the reply row's config over the last exchange; on success feed the result to `EditorView::set_autosuggestion` (Tab/→ accept via `editor_view:insert_autosuggestion`, §33); any keystroke clears (§34); never auto-send (§33). Off-thread, best-effort, gated by the §30 toggle; unauth/failure/non-empty → nothing (§35). **Acceptance:** with the toggle on and a valid reply-row provider, a suggestion appears as ghost text after a turn, Tab fills it, typing dismisses it, and it never sends itself or hangs. PRODUCT §30–§36.

### 16f — Terminal AI command suggestions
Add a `SuggestionProvider`-backed impl behind the terminal `Autosuggester` seam as a **fallback** below instant history (§41). On a typing-pause debounce with no history match and the §40 toggle on, generate via the terminal row's config off-thread and `set_autosuggestion` an `AutosuggestionType::Command`; accept via the existing key; dismiss-on-type; unauth/failure/edit-in-flight → nothing; never auto-run (§43). Respect the subscription-is-heavy constraint (resident-CLI provider is debounced-on-pause; API-key provider is the fast path). **Acceptance:** with the toggle on, typing a novel command with no history match yields an AI ghost suggestion after a pause, accepted with the autosuggestion key, never blocking input or running a command; instant history suggestions are unchanged. PRODUCT §40–§43.

## Testing

| PRODUCT § | Test | Kind |
|---|---|---|
| §1–§4 | Agent page registers; selector lists Claude(enabled)/Codex/Gemini(disabled); disabled select is a no-op | unit (view) |
| §5 | capability-aware render: Claude shows perm-modes+effort; a capability-less test backend hides them | unit |
| §6–§9, §11 | auth-status from install/login probes (mocked); probe off-thread, non-quota | unit |
| §10, §12–§13 | key save → keychain (mocked) + presence flag; masked field; clear → fallback to local login | unit |
| §14–§19 | new-pane precedence launch → Chat-row → fallback; Chat-row authoritative over last-used; invalid model degrades; open panes unaffected | unit + manual (launch app) |
| §15 | perm-mode default offers exactly {default, acceptEdits, plan, bypassPermissions} | unit |
| §20–§22 | rows persist independent {provider,model,effort}; "Default" = inherit; unauth provider warns | unit |
| §23–§26 | non-secret config round-trips; corrupt/absent → safe defaults; no key outside keychain; no key in telemetry | unit |
| §30–§36 | reply ghost-text: appears on Ended+empty+idle; Tab accepts (no send); keystroke clears; unauth/failure → nothing | unit + manual |
| §40–§43 | terminal AI suggest: fallback below history; debounced; accepted via key; never auto-runs; history layer unchanged | unit + manual |

**Manual/launch checks** (per `twarp_keybinding_menu_mechanism` — UI/keybinding/spawn/ghost-text changes must be verified in the running app): Settings → Agent; change Chat model/effort/mode and confirm a freshly-typed `claude` pane starts there, overriding the last-cycled pill; save an API key and confirm it lands in Keychain (`security find-generic-password`) and not the TOML; enable reply suggestions and confirm ghost text + Tab-accept + dismiss-on-type; enable terminal suggestions and confirm the debounced fallback behind history.

## Feature flag

The Agent **page** ships unflagged (provisional). Each **suggestion consumer** is gated by its own settings enable toggle (§30, §40), off by default — so the generators are dark until a user opts in, independent of any compile-time flag.

## Open decisions (for review)

1. **Keychain crate.** `security-framework` (mac-native, matches mac-first) vs `keyring` (cross-platform). Grep for existing usage first; default `security-framework`.
2. **Settings storage shape.** `define_settings_group!` for the flat parts + a nested blob for `actions{}`, vs a `PersistedData` field. Pick per the macro's handling of the nested map.
3. **`claude_session_defaults` fate.** 16a makes the Chat row authoritative for new-pane *starting* values; last-used is demoted to pane *restore* only. Confirm we don't drop the table entirely (7m persistence relies on it — check before removing).
4. **Login probe command per backend.** The exact non-interactive, zero-quota probe for `claude` (and later codex/gemini) must be empirically verified against the pinned CLI before 16b.
5. **Resident-CLI vs `-p` for `SuggestionProvider`.** A persistent `claude` process (lower per-call latency, mirrors the pane driver) vs a fresh `-p` per suggestion (simpler, higher latency). Default: resident for terminal (high frequency), `-p` acceptable for reply (once per turn).
