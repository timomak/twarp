---
name: fleet
description: Check the twarp dev-fleet status and, if nothing is already running, start a fleet run (parallel dual-machine workers → gate → UX gate → auto-merge). Use when the user runs /fleet or asks to start/check the fleet.
---

# fleet

One-command entry point to the twarp dev fleet (`fleet/fleet.py`). The user runs `/fleet` and this
skill **reports status first, then starts a run only if nothing is already in progress.**

Always run from the repo root: `/Users/thirdfacedev/Development/twarp`.

## Workflow for `/fleet` (no args)

0. **Pull the next roadmap sub-task** (auto-bridge from the roadmap):
   ```bash
   python3 fleet/fleet.py roadmap-sync
   ```
   If the active roadmap feature is `impl-pending`, this adds its next unchecked sub-phase to the
   queue (routed to other-mac/Codex). For any other phase it pulls nothing and prints what's needed
   (e.g. "review/merge the spec PR" — **specs stay human**). Report this line.
1. **Is a run already in progress?**
   ```bash
   pgrep -fl "fleet/fleet.py run"
   ```
2. **Get the ledger:**
   ```bash
   python3 fleet/fleet.py status
   ```
   The last line shows `eligible now: [...]` — items ready to dispatch.
3. **Decide and act:**
   - **A run is already alive** (step 1 found a process) → report status + "a run is already in
     progress" and **stop**. Do not start a second one.
   - **No process, but items are stuck in-flight** (status shows `leased`/`building`/`gating`/
     `gated`/`merging`) → a previous run was interrupted. Report it and tell the user to reset with
     the reset snippet below; **do not auto-start**.
   - **No process and `eligible now` is non-empty** → start a run **in the background**:
     ```bash
     cd /Users/thirdfacedev/Development/twarp
     nohup python3 fleet/fleet.py run > fleet/runs/run.log 2>&1 &
     ```
     Then report what was launched (the eligible items) and that it will author in parallel on both
     machines, gate, UX-gate (for `ux:true` items), and auto-merge whatever passes. Set up a
     background watcher on `fleet/runs/run.log` for the `=== run complete ===` marker so you can
     report the outcome when it finishes.
   - **No process and `eligible now` is empty** → everything is merged or blocked. Report that
     there's nothing to do and that work is added by dropping items into `fleet/queue.json`
     (`id`, `node`, `touches`, `depends_on`, `task`, `verify`, optional `"ux": true`). **Stop.**

## `/fleet status`

Run steps **1–2 only** (skip step 0 — `status` is read-only and must not pull/modify the queue) and
report. **Never start a run**, even if items are eligible.

## Reset a stuck/interrupted run

If items are stranded in-flight (process died mid-run), clear them back to `queued`:
```bash
python3 - <<'PY'
import sys; sys.path.insert(0,'fleet'); from fleet import load,save,INFLIGHT
q=load()
for it in q['items']:
    if it['status'] in INFLIGHT: it['status']='queued'; it.pop('branch',None)
save(q); print('reset in-flight items to queued')
PY
```

## Status report format

Keep it short:
```
fleet status
  Running:  <yes (pid) | no>
  Eligible: <ids | —>
  In-flight:<ids | —>
  Merged:   <recent ids>
  Action:   <started run on [...] | reported only | nothing to do>
```

## Rules

- **Never start a second concurrent run.** One `fleet.py run` at a time (the build node has one
  screen and one cargo cache).
- **`/fleet status` never starts anything.**
- The fleet **auto-merges green PRs to master** — that is intended (gates are the safety net). Don't
  add confirmation prompts.
- Don't edit `queue.json` item definitions on the user's behalf unless they ask — adding work is a
  human decision.

## What the fleet does (for reference)

`fleet.py run` → dispatch (file-disjoint, dependency-aware) → parallel workers (Claude local +
Codex on other-mac) → functional gate on the build node → UX vision gate for `ux:true` items →
bors-style speculative-merge → auto-merge. See `fleet/README.md`.
