# Feature 26 — Sessions & Projects MCP server (`twarp-sessions`)

## Summary

A third built-in MCP server, `twarp-sessions`, that lets agents create chat sessions and projects in twarp, monitor sessions (list, read transcripts, stream updates), and wait on completion. It is exposed on two surfaces: auto-injected into every agent pane (like `twarp-browser`), and an opt-in token-gated localhost listener so external agents — primarily the dev fleet's UX gate, replacing CGEvent injection — can drive twarp. Claude and Codex sessions are first-class on every tool.

## Problem

Only agents running inside twarp panes get twarp's built-in MCP tools, and none of those tools touch sessions: an agent cannot spawn a sibling chat, watch another session, or organize work into projects. Externally, the fleet drives twarp through fragile accessibility-based event injection. Both needs are served by one session-control surface.

## Goals / Non-goals

Goals: session create/list/read/watch/wait, project create/list, both surfaces, Claude+Codex parity (including injecting built-in servers into Codex sessions, which today receive none).

Non-goals (deferred): `send_message`/steering of an existing session; headless (pane-less) sessions; lazy tool loading and a docs filesystem (future feature 27); auth beyond a single bearer token; any cloud/remote transport — external surface is localhost only.

## Behavior

The "user" here is an agent (MCP client): either an agent inside a twarp pane, or an external local process holding the token.

### Tool surface

1. Both surfaces expose the same seven tools: `create_chat`, `list_sessions`, `get_transcript`, `wait_for_completion`, `watch_session`, `list_projects`, `create_project`. There is no read-only tier; safety is carried by caps and provenance (see 22–26), not by withholding tools.

### Listing and status

2. `list_sessions` returns every live agent pane and, when `include_past: true`, past sessions from both providers' stores. Each entry has: `session_id`, `provider` (`claude` | `codex`), `cwd`, `title`, `status`, `is_live`, last-activity timestamp, and `project` (name, or null) for live sessions.
3. `status` is one of exactly: `running` (streaming or background work active), `needs_input` (blocked on a permission request or question), `done_ok`, `done_error`, `idle` (live pane, no turn in progress, not in a done state — e.g. freshly opened or after the user cleared attention). Past (non-live) sessions always report `idle`.
4. The status reported by `list_sessions` always matches what the twarp UI shows for that pane (tab dot / attention state). The two must never disagree — they derive from the same signal.
5. Filters: `list_sessions` accepts optional `status`, `provider`, and `cwd` filters. An empty result is an empty list, not an error.

### Transcripts

6. `get_transcript(session_id, since_index?)` returns the session's messages as an ordered list of items, each with a stable, monotonically increasing `index`, a `role`/kind, and text content. `since_index` returns only items after that index; passing the latest index returns an empty list.
7. Transcripts are identical in shape for Claude and Codex sessions. Live sessions read current in-memory state; past sessions read stored history. A `session_id` that exists in neither returns a not-found error.
8. Item indices are stable within a session lifetime: an item's index never changes, and items are never reordered. A client polling with `since_index` never misses an item and never receives a duplicate.

### Watching and waiting

9. `watch_session(session_id)` subscribes the calling connection to that session: subsequent transcript items and status changes arrive as MCP notifications on the same connection, without polling. Multiple watchers on one session all receive every event.
10. `wait_for_completion(session_id | "any", timeout_seconds)` blocks until the target session (or any live session, for `"any"`) transitions to `done_ok`, `done_error`, or `needs_input`, then returns the final status plus the last assistant message's text. On timeout it returns a distinct `timed_out` result, not an error.
11. Completion for `wait_for_completion` matches the app's own "deferred completion" semantics: a session still running background scripts or sub-agents is not complete until they finish — the same rule that gates the UI checkmark and notification.
12. If a watched/waited session's pane is closed or its process exits, watchers receive a terminal `closed` event and `wait_for_completion` returns with status `done_error` (reason: closed) rather than hanging.

### Creating sessions

13. `create_chat(prompt, cwd, provider?, model?, project?)` opens a new agent pane in a new tab, submits `prompt` as the first user message, and returns the new `session_id` immediately (it does not wait for the reply — combine with `wait_for_completion`). `provider` defaults to the app's configured default; `model` defaults per provider settings.
14. The created pane is a fully normal agent pane: visible, focusable, persisted/restorable, and indistinguishable in capability from a user-opened one — except for provenance (22).
15. `cwd` must be an existing directory; otherwise the tool errors and nothing is created. If `project` is given and matches no existing project, the tool errors (it does not implicitly create one).
16. A `create_chat` from an in-pane agent records the creating session as the new session's parent; external creations record the external consumer as origin.

### Projects

17. `list_projects` returns each sidebar project's `name`, `color` (always null — projects have no persisted color), `cwd`, and the count of its sessions.
18. `create_project(name, cwd, color?)` creates a sidebar project exactly as if made through the UI and returns it. A project already registered for the same folder is a duplicate and errors. twarp projects have no persisted color (color is a per-tab property), so `color` is accepted but ignored and the returned `color` is null — same as `list_projects` (17). The new project appears in the sidebar immediately.

### Surfaces, exposure, and auth

19. In-app surface: `twarp-sessions` is auto-injected into every Claude session's MCP config alongside `twarp-browser` and `twarp-computer-control`, with no user setup.
20. Codex parity: Codex sessions receive all built-in servers (`twarp-sessions`, `twarp-browser`, `twarp-computer-control`) just like Claude sessions. Every tool in this spec behaves identically when the caller or the target session is Codex.
21. External surface: off by default. A setting ("Allow external agents to control sessions") enables a localhost-only listener on a fixed, configurable port. Enabling generates a bearer token stored in an owner-only-readable file whose path the settings page shows; a "regenerate" action invalidates the old token immediately. Every request without a valid token is rejected; the in-app per-session servers remain tokenless and unreachable off-machine, as today. Disabling the setting stops the listener and severs live watchers (12).

### Safety and provenance

22. A pane created via `create_chat` shows a provenance badge in its header identifying the creator (the parent session's title, or an "external: …" label — the bearer token carries no consumer identity today, so external creations are labeled "external"). The badge persists for the pane's lifetime and across restore.
23. Spawn cap: at most N concurrently *running* created sessions (default 4, configurable). At the cap, `create_chat` fails immediately with a distinct at-capacity error naming the limit; nothing is queued.
24. Spawn depth: each created session records depth = parent's depth + 1 (user- or external-created = 0/1). Depth ≥ 3 is refused with a distinct error, so a runaway agent-spawning chain halts by construction.
25. Session-scoped reads behave like the other built-in servers: tools are stamped with the calling session, and monitoring another session never steals its focus, moves its pane, or marks its attention state as seen.
26. Nothing in this feature auto-approves anything inside created sessions: a spawned session's permission prompts behave exactly as in a user-opened session, and a spawned session blocked on `needs_input` surfaces through status/notifications like any other.

### Errors and edge cases

27. All tool errors are structured and distinguishable: not-found, invalid-argument, at-capacity, depth-exceeded, timeout, unauthorized, and surface-disabled. An agent can branch on which occurred.
28. Concurrent `create_chat` calls racing the spawn cap: at most N succeed; losers get the at-capacity error. Never N+1 running spawned sessions.
29. App shutdown / pane close mid-operation: in-flight `wait_for_completion` and watches resolve per (12); no tool call hangs indefinitely.
30. The external listener rebinds on app restart with the same port and token; a port conflict surfaces as a settings-page error state, not a silent failure.

## Fleet adoption (validating consumer)

The dev fleet's UX gate is the first external consumer: it replaces uidrive CGEvent injection with `create_chat` + `wait_for_completion` + `get_transcript`, keeping computer-control screenshots for pixel assertions. The feature is not done until a fleet UX-gate round passes end-to-end over this surface.
