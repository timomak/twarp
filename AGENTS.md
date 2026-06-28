# twarp — Codex worker notes (other-mac fleet node)

You are a **worker** in twarp's parallel dev fleet, running on `other-mac` via Codex+Foundry.
Your job: implement **one assigned sub-phase** from already-merged specs, build it, and push a
branch. You do **not** merge, do **not** open PRs against upstream, and do **not** pick what to
work on — that's the dispatcher's job.

## Git remotes (critical)

- `origin` → `timomak/twarp` (the fork — all branches/PRs go here)
- `upstream` → `warpdotdev/warp` (**read-only — never push, comment, or open issues/PRs here**)

Push your work branch to `origin` only. Branch name: `twarp-<feature-id>` (e.g. `twarp-11-git-blame`),
based off the latest `origin/master`. **Never push to `master`. Never merge.** A separate
merge-queue supervisor tests and merges; opening the PR happens from the primary Mac (this node has
no `gh`).

## What a task looks like

The dispatcher assigns you one sub-phase. Specs live at `roadmap/<NN-feature>/PRODUCT.md` and
`TECH.md` (already merged to master — read them, don't rewrite them). Implement **only** the assigned
sub-phase. Keep the diff scoped to the files the sub-phase needs. Each PRODUCT.md ends in a
`## Smoke test` checklist — your change must satisfy it.

## Build & check

- Build: `cargo build --bin warp-oss` (default dev binary; `warp` adds dogfood/preview).
- This node is **headless** — do not attempt to launch the GUI or run real-display tests. Run only
  headless/offscreen integration tests and unit tests. Real-display UX gates run on the primary Mac.
- Format/lint: `cargo fmt -- --check`, `cargo clippy --workspace -- -D warnings`. Note this node is
  missing clang-format, wgslfmt, and nextest, so `./script/presubmit` will not fully run and plain
  `cargo test` can report false failures vs nextest — get the Rust build + clippy + fmt green and
  leave the rest to the supervisor's gates.

## Hard rules

- One sub-phase per branch. Stay in scope.
- Never touch `upstream` (`warpdotdev/warp`) in any way.
- Never merge, never push to `master`, never edit `ROADMAP.md` order.
- If specs are ambiguous, make the smallest reasonable choice and note it in the branch's commit
  body — do not block waiting for input (this node runs unattended).
- Feature-flag incomplete work where the feature uses a flag, so a partial merge stays dark.

## Spec storage convention

twarp roadmap specs live at `roadmap/<NN-feature>/PRODUCT.md` / `TECH.md` (overrides the repo's
default `specs/<linear-ticket>/` layout).
