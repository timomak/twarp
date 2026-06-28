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
| `python3 fleet/fleet.py run` | **Continuous batch loop**: fill up to `batch` items → author in parallel → drive each PR to green + architect-approved → merge-queue → refill → repeat until drained |
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

## Parallel batch loop + per-PR iterate

`fleet.py run` is a continuous loop (`config.batch`, default 5):

1. **Fill a batch** — up to `batch` ready items (deps met, file-disjoint), topped up from the roadmap.
2. **Author in parallel** — one agent session per item across Claude (local) + Codex (other-mac). N
   authors run at once; **all building/testing serializes through the single build node** (`_gatelock`)
   — that's the "one queue gate always running."
3. **Drive each PR to mergeable** (`iterate`) — a PR is opened immediately, then per item:
   `gate (build+test) → if red, fix-agent revises → re-gate → when green, staff-architect review →
   if changes requested, fix → re-gate → when green + approved → ready`. Up to `MAX_ROUNDS` attempts.
   The PR auto-updates every round until tests are green **and** the staff-architect approves.
4. **Merge queue** — ready PRs go through the speculative-merge gate and auto-merge, serialized.
5. **Report + refill** — print the batch result, pull the next batch, repeat **until the queue and
   roadmap are drained**.

So: N parallel authors, one serialized gate, every PR green-and-architect-approved before it merges.
Throughput is bounded by the single build node, not by the number of authors.

## Roadmap bridge (auto-pull)

`fleet.py run` (and `/fleet`) call `roadmap_sync()` first, which reads `roadmap/ROADMAP.md`'s active
feature and its `STATUS.md`:
- **Specs stay human.** If the feature isn't `impl-pending` (e.g. `spec-in-review`, `not-started`),
  it pulls **nothing** and prints what's needed — spec writing/review stays with `/twarp-next` + you.
- **Impl auto-flows.** When the feature is `impl-pending`, it pulls the **next unchecked sub-phase**
  from `STATUS.md` into the queue — routed to **other-mac/Codex** (sparing the Claude sub), with a
  task that reads the merged `PRODUCT.md`/`TECH.md` and ticks the sub-phase checkbox on completion.
  One sub-phase at a time (they're sequential and share files).

`python3 fleet/fleet.py roadmap-sync` runs just the bridge (debug).

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
- **Batch size** is `config.batch` (default 5); authors run in parallel, but builds/tests serialize
  on the single build node (`_gatelock`), so the gate is the throughput ceiling, not the author count.
- The per-PR loop retries up to `MAX_ROUNDS` (default 4); an item that can't reach green+approved is
  marked `exhausted` (its PR stays open for a human). `needs-rebase` items also wait for a human.
- `fleet/runs/` (prompts + per-item logs) is gitignored.
