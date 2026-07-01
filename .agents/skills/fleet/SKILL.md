---
name: fleet
description: Check the twarp dev-fleet status and, if nothing is already running, start a fleet run (parallel dual-machine workers → gate → UX gate → auto-merge). Use when the user runs /fleet or asks to start/check the fleet.
---

# fleet

One-command entry point to the twarp dev fleet (`fleet/fleet.py`). The user runs `/fleet` and this
skill **reports status first, then starts a run only if nothing is already in progress.**

Always run from the repo root: `/Users/thirdfacedev/Development/twarp`.

## Pods & run modes (node-pod model)

A **pod** is one machine = N builder sessions + its own gate (builds/tests locally; gates run serial
within a pod, parallel across pods). Two modes:

- **Default (`/fleet`)** — the whole loop runs **on other-mac** (single codex pod, 2 builders + 1
  gate). This Mac stays idle. Launch it over SSH with the pod env sourced:
  ```bash
  ssh other-mac 'source ~/.config/twarp-fleet/env; cd ~/Development/twarp && \
    nohup python3 fleet/fleet.py run --self other-mac > fleet/runs/run.log 2>&1 & echo started'
  ```
  **Requires `GH_TOKEN` on other-mac** (so its `gh` can open/merge PRs). If absent, the run authors+
  gates but can't merge — check with `ssh other-mac 'source ~/.config/twarp-fleet/env; gh auth status'`
  and, if unset, tell the user to add a `repo`-scoped PAT to `~/.config/twarp-fleet/env` on other-mac.
- **Both machines (`/fleet --both` or `/fleet both`)** — the loop runs **here on this Mac** with two
  pods (local=claude here + other-mac=codex over SSH) → **4 builders / 2 gates**. `gh` runs here
  (already authed), so no token needed. Launch locally:
  ```bash
  cd /Users/thirdfacedev/Development/twarp
  nohup python3 fleet/fleet.py run --self local --both > fleet/runs/run.log 2>&1 &
  ```

For the default mode, `pgrep`/`status`/`run.log` live **on other-mac** — run those over SSH. For
`--both`, they're local (this Mac).

## Workflow for `/fleet` (no args)

0. **Pull the next roadmap sub-task** (auto-bridge from the roadmap):
   ```bash
   python3 fleet/fleet.py roadmap-sync
   ```
   This bridge is **fully autonomous — no human gate**. It advances the active roadmap feature one
   step per call and enqueues the matching fleet item (authored + opposite-model-reviewed +
   auto-merged like any other item):
   - `not-started` / `spec-pending` → a `<feat>-spec` item that writes PRODUCT.md + TECH.md (with a
     `## Smoke test` section) and flips the feature to `impl-pending`.
   - `impl-pending` → the next unchecked sub-phase.
   - feature fully merged → a `<feat>-advance` item that points `Currently active:` at the next
     `not-started` feature **in ROADMAP table order**.

   The owner steers only by editing `roadmap/ROADMAP.md` — the table order and the `Currently
   active:` pointer; the fleet never reorders the table. Note: when a feature finishes, auto-advance
   picks the **lowest-numbered remaining `not-started`** feature, which is currently `09-rebrand`
   (the `**`-touching barrier). To build something else next, set `Currently active:` (or reorder the
   table) before the active feature merges. Report the printed line.
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
   - **No process and `eligible now` is non-empty** → start a run **in the background** using the
     launch command for the requested mode (see **Pods & run modes** above): default = on other-mac
     over SSH; `--both` = locally with both machines. Then report what was launched, and that it runs
     as a **continuous batch loop**: authors items in parallel across pods (up to total builder
     capacity), drives each PR to green + staff-architect-approved (auto-fixing until it passes, with
     the review done by the *opposite* model from the author), auto-merges through the speculative
     gate, then refills and repeats **until the queue + roadmap are drained**. Set up a background
     watcher on `run.log` for the `=== run complete ===` marker (and the per-batch
     `=== batch N done ===` lines) so you can report progress/outcome. For default mode the watcher
     must read other-mac's `run.log` over SSH.
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

- **Never start a second concurrent run.** One `fleet.py run` at a time across the whole fleet — a
  pod has one cargo cache and one display, and two loops would double-lease items. Check for an
  existing process on *both* the default (other-mac, over SSH) and `--both` (local) launch points.
- **`/fleet status` never starts anything.**
- The fleet **auto-merges green PRs to master** — that is intended (gates are the safety net). Don't
  add confirmation prompts.
- Don't edit `queue.json` item definitions on the user's behalf unless they ask — adding work is a
  human decision.

## What the fleet does (for reference)

`fleet.py run` → dispatch (file-disjoint, dependency-aware) → assign items across active pods →
parallel workers (codex on other-mac / claude on local) → per-pod functional gate (parallel across
pods) → **dynamic UX gate** for `ux:true` items (on a display pod) → opposite-model staff-architect
review → bors-style speculative-merge → auto-merge (`gh` on the self pod). `config.nodes` defines
pods; `pods_default`/`pods_both` select the active set. See `fleet/README.md`.

## Dynamic UX gate (`ux:true` items)

For `ux:true` items the gate doesn't just screenshot the bootstrap screen — it **builds the PR's
`warp-oss`, launches it on the display pod's real display, and a Claude agent drives the live app
(screenshot → reason → click/type → observe, computer-use style) to verify the feature actually
works against acceptance criteria** (`ux_criteria` on the item, else the feature's PRODUCT.md
`## Smoke test`). A `regression` verdict routes back to the fix-agent; evidence (screenshots +
transcript) lands in `fleet/runs/ux_<id>/`. Code: `ux_drive_gate()` in `fleet.py`. Run it standalone
with `python3 fleet/fleet.py uxdrive <id>`.

The gate needs BOTH an inject side and a capture side on the display pod — each is an in-session
launchd agent holding one macOS TCC grant (a process over SSH is sshd-attributed and holds neither):

- **Inject (act):** `~/.local/bin/uidrive` (CGEvent injector) runs in-session via the
  `com.twarp.uidrive` LaunchAgent as the **`UidriveAgent.app`** bundle. SSH queues commands with
  `uinject`. Needs **Accessibility**.
- **Capture (see):** `~/.local/bin/uicapture` runs in-session via the `com.twarp.uicapture`
  LaunchAgent as the **`UicaptureAgent.app`** bundle; `~/.local/bin/uishot` routes captures through it
  (writes the path to `~/.uicapture/in`, polls for the file). Needs **Screen Recording**. A bare
  `screencapture` over SSH silently returns a privacy-limited desktop-only frame (no app windows) —
  this is the #1 gotcha.
- Screenshots are Retina (scale 2) — coords are pixels; divide by 2 for the point coords `uidrive`
  wants.
- **TWO one-time human GUI grants on the display pod** (System Settings → Privacy & Security):
  **Accessibility → UidriveAgent** and **Screen Recording → UicaptureAgent**. Verify:
  `ssh other-mac '~/.local/bin/uidrive trusted'` → `trusted=true`, and
  `ssh other-mac "printf prompt > ~/.uicapture/in; sleep 1; tail -1 ~/.uicapture/log"` → `granted=true`.
  **If either is missing the gate auto-falls-back to the bootstrap-screenshot gate** (degrades, never
  blocks). Each grant is pinned to its bundle signature — **never rebuild/re-sign either binary after
  granting** or the grant silently stops matching (fix: `tccutil reset <Accessibility|ScreenCapture>
  <bundle-id>`, freeze the binary, re-fire its `prompt`, re-toggle).
- **Startup permission dialogs:** a freshly-launched `warp-oss` bundle can block on macOS prompts
  (e.g. *"WarpOss would like to access data from other apps"*) before showing a window — the Claude
  driver clicks **Allow** to get past them, so this is handled by the loop, not a launch failure.
