# twarp dev fleet

Parallel, multi-machine implementation of the twarp roadmap. Replaces the linear one-feature-at-a-time
loop with a dispatcher + worktree workers + a bors-style merge-queue supervisor that auto-merges
green branches.

```
                          ┌─────────────── this Mac (orchestrator) ───────────────┐
  fleet/queue.json  ───▶  │  dispatch  →  workers (parallel)  →  merge-queue       │
   (work ledger)          │     │            │      │              supervisor       │
                          │     │       local Claude │             (auto-merge)     │
                          └─────┼────────────┼───────┼───────────────┼─────────────┘
                                │            │       │               │
                                ▼            │       ▼               ▼
                        disjoint scheduling  │   other-mac ◀── BUILD NODE (gate + speculative-merge gate)
                        (touches / deps /    │   Codex via Foundry          + screen semaphore for UX gates
                         barrier)            └── pushes branch
```

## Roles

| Command | What it does |
|---|---|
| `python3 fleet/fleet.py status` | Show the ledger + which items are eligible right now |
| `python3 fleet/fleet.py run` | Full loop: dispatch → author (parallel) → gate → merge-queue auto-merge |
| `python3 fleet/fleet.py worker <id>` | Author one item on its node (debug) |
| `python3 fleet/fleet.py gate <id>` | Build + targeted tests for a branch on the build node (debug) |
| `python3 fleet/fleet.py supervise <id>` | Speculative-merge + gate a branch (debug) |
| `python3 fleet/fleet.py uxgate [test]` | Render twarp on the build node's real display, capture a screenshot, diff vs golden |

## UX / visual gate

`uxgate` renders twarp on **other-mac's built-in display** (Option A — lid open, no extra hardware),
runs an integration test that bootstraps the UI (`test_video_recording`), captures a real Retina
screenshot, pulls it here, and compares against the golden baseline in `fleet/golden/`:
- first run → `golden-saved` (baseline stored)
- byte-identical → `pass`
- changed → `review` (a vision agent inspects the PNG for layout/contrast/clipping regressions)

The screenshot capture works **over SSH** because other-mac has an active GUI console session +
`caffeinate` keeps the display awake. The pixel `cmp` is a coarse change-detector; the actual visual
judgment is done by a vision agent on the returned PNG (and `cmp` should be upgraded to a perceptual
diff to tolerate benign AA noise).

## The work ledger (`queue.json`)

Each item declares:
- `node` — `other-mac` (Codex via Foundry), `local` (Claude), or `any`
- `depends_on` — items that must be `merged` first
- `touches` — file globs it will change; the dispatcher only co-schedules items whose `touches` are
  **disjoint** (so two workers never collide on the same files)
- `barrier` — `true` drains the fleet and runs solo (e.g. `09-rebrand`, which renames every crate)
- `task` — the prompt handed to the worker; `verify` — the gate command (build + targeted tests)

## Why it's safe to auto-merge

Two PRs can each be green alone yet break `master` together (semantic conflict). The supervisor
guards against this: before merging branch B it **re-runs the gate on `master + B`** (speculative
merge) on the build node, and only auto-merges if that combination is green — bors / GitHub-merge-queue
discipline. Disjoint scheduling makes conflicts rare; the speculative gate catches the rest.

## Build node

`other-mac` is the single build/test node (warm cargo cache + working toolchain; this Mac can't run
the full toolchain). All building, gating, and speculative-merge gating happen there regardless of
which node authored the branch. See the `twarp_fleet_other_mac` memory for its setup.

## Proven

`fleet.py run` was validated end-to-end (2026-06-28): two disjoint seed tasks authored in parallel
(Codex on other-mac + Claude locally), both gated green, both speculative-merged and auto-merged to
`master` as PRs #101 and #102, independently re-verified green.

## Status / not yet built

- **UX gate** is wired (`uxgate`, real display on other-mac) and produces a golden-compared
  screenshot. Still to harden: call the vision agent automatically on a `review` verdict, upgrade the
  byte `cmp` to a perceptual diff, and fold `uxgate` into `run` (per-item, gated by a screen
  semaphore) rather than a standalone command.
- **Speculative depth = 1** (serialized merges). No N-deep speculative train yet.
- Concurrency is capped in `queue.json` (`config.concurrency`). Builds serialize on the build node's
  cargo lock (shared `CARGO_TARGET_DIR`).
- `fleet/runs/` (prompts + per-item logs) is gitignored.
