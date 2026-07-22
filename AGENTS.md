# twarp — repository guidance

These instructions apply to every Codex task in this repository. Do not infer a fleet role, node,
or workflow from the checkout name or machine. The fleet harness supplies its own worker policy and
assignment when it launches a fleet task.

## Git remotes and delivery

- `origin` is `timomak/twarp`, the writable fork. Branches, pull requests, reviews, and merges belong
  there.
- `upstream` is `warpdotdev/warp` and is strictly read-only. Never push to it or create, comment on,
  review, or merge issues or pull requests there.
- Never push directly to `master`. When the user explicitly asks to deliver a change, use a feature
  branch and a pull request against `origin/master`; merging that pull request is allowed when the
  user requests it and the required checks pass.
- Preserve unrelated user changes and keep each change scoped to the requested work.

## Build and validation

- Default application build: `cargo build --bin twarp-oss`.
- Rust formatting and lint checks: `cargo fmt -- --check` and
  `cargo clippy --workspace -- -D warnings`.
- Prefer targeted tests for the changed crate or module before broader checks. Report baseline or
  environment failures separately from failures caused by the change.
- Do not assume the current machine is headless or missing tools; inspect its capabilities when
  those details matter.

## Project conventions

- Roadmap specs live at `roadmap/<NN-feature>/PRODUCT.md` and `TECH.md`.
- Do not reorder `roadmap/ROADMAP.md` unless the task explicitly requires roadmap maintenance.
- Feature-flag incomplete work when the feature already uses a flag so partial changes remain dark.

## Fleet tasks

Fleet-only role restrictions live in `fleet/WORKER.md` and are injected by `fleet/fleet.py`. They do
not apply to ordinary interactive tasks merely because this file exists at the repository root.
