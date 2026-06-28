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
screenshot, pulls it here, and has a **vision agent** compare it to the golden baseline in
`fleet/golden/`:
- first run → `golden-saved` (baseline stored)
- `pass` / `regression` — a headless `claude -p` views BOTH the new shot and the golden and judges
  whether there's a user-visible regression (misaligned/clipped/missing UI, broken layout, wrong
  colors). It ignores benign noise (anti-aliasing, cursor blink, sub-pixel shifts) that a byte
  compare would false-flag.

The capture works **over SSH** because other-mac has an active GUI console session + `caffeinate`
keeps the display awake. Captures hold a **screen semaphore** (`_screenlock`) so only one real-display
render runs at a time. Items in `queue.json` flagged `"ux": true` get this gate automatically inside
`fleet.py run` after their functional gate — a `regression` verdict blocks the merge.

Validated end-to-end: a fresh capture vs golden → `pass`; a deliberately broken (cropped) image →
`regression`, correctly described.

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

- **UX gate** is wired and hardened: vision-agent comparison (not byte `cmp`), screen semaphore,
  and per-item integration into `fleet.py run` (`"ux": true` items block merge on a `regression`).
  Remaining polish: per-item UX *tests* (each item drives twarp to the screen it changed, instead of
  the generic bootstrap shot) and its own golden.
- **Speculative depth = 1** (serialized merges). No N-deep speculative train yet.
- Concurrency is capped in `queue.json` (`config.concurrency`). Builds serialize on the build node's
  cargo lock (shared `CARGO_TARGET_DIR`).
- `fleet/runs/` (prompts + per-item logs) is gitignored.
