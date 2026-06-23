---
name: staff-architect-review
description: Review the current change at a staff/principal-engineer altitude — architecture, blast radius, fork discipline, rollout/reversibility, and long-term maintainability — instead of line-level bugs. Use when the user runs /staff-architect-review or asks for an architectural / design review of a diff, branch, or PR before merge.
---

# Staff-architect review

A high-altitude design review of the current change, written from the perspective of a staff/principal engineer who owns the long-term health of the codebase. It answers **"is this the right shape of change, and what does it cost us later?"** — not "is line 42 buggy?".

This skill **complements, it does not replace,** the tactical reviewers:

- `/code-review` — correctness bugs, reuse, simplification, efficiency in the diff.
- `review-pr` / `review-pr-local` — inline PR feedback with suggestion blocks.

Run those for line-level defects. Run **this** for the questions a senior reviewer asks before approving a design: does this belong here, what else does it touch, can we undo it, and will we regret it in six months. If you find a concrete line-level bug while reviewing, note it — but the deliverable here is the architectural assessment, not a list of nits.

## Scope of the change to review

Default target is the current local diff against `master`:

```bash
git fetch origin master --quiet
git diff --stat origin/master...HEAD
git diff origin/master...HEAD
```

Argument handling:

- `/staff-architect-review` (no args) — review the current branch vs `origin/master`.
- `/staff-architect-review <PR#>` — check out / fetch the PR diff (`gh pr diff <PR#> --repo timomak/twarp`) and review that.
- `/staff-architect-review <path…>` — restrict the review to the given files/dirs.

Read enough surrounding code to judge the design, not just the diff hunks. A staff review that only looks at changed lines misses the point — the value is in how the change sits in the system.

## What to evaluate

Work through these lenses. Skip a lens explicitly if it doesn't apply rather than padding the report.

### 1. Architecture & boundaries
- Does this change live at the right seam? Is logic landing in the layer that should own it (model vs view vs input vs persistence), or is it leaking across boundaries?
- Does it introduce a new abstraction, trait, crate dependency, or indirection? Does that earn its keep, or would inlining / reusing an existing primitive be simpler?
- Is there an existing pattern in the codebase this should mirror instead of inventing a parallel one? (e.g. a new pane should mirror `NetworkLogPane`; a new terminal-command trigger should mirror the `OpenCodeInWarp` chain.)
- Could this be meaningfully smaller? Name the smaller design if one exists.

### 2. Blast radius & risk
- What else touches the code paths, types, or state this modifies? Who breaks if the contract shifts?
- **Locking & concurrency:** flag nested or redundant `TerminalModel` locking, long-held lock scopes, and async lifecycle gaps (tasks that outlive their owner, missing cancellation). These are the recurring footguns in this codebase.
- **Persistence & migrations:** if it changes a persisted shape (settings, session state, DB rows), is old data still readable? Is there a migration or a default?
- **Failure modes:** what happens on the unhappy path — IO error, missing binary, absent metadata, a `claude` subprocess that dies? Does it degrade gracefully or hang/panic?

### 3. Fork discipline (twarp-specific, weight this heavily)
twarp is a long-lived fork of `warpdotdev/warp`. Every line that diverges from upstream is a line that must be re-reconciled on each upstream merge. A staff reviewer protects against *gratuitous* divergence.
- Does this change modify upstream files in a way that will collide on the next merge? Could the same result be achieved in a twarp-owned file (new module, additive hook) instead of editing the middle of an upstream one?
- If an upstream file *must* change, is the edit minimal and localized (one call site, one additive arm) rather than a sprawling rewrite?
- Is this re-implementing something upstream already provides, or fighting an upstream pattern instead of extending it? Note when the change could plausibly be upstreamed as a generic primitive (the twarp-next upstream-assessment path), stripped of AI-removal framing.

### 4. Rollout & reversibility
- Should this be behind a `FeatureFlag` and channel-gated, or is it safe to ship unconditionally? Prefer runtime `FeatureFlag::X.is_enabled()` over `#[cfg(...)]` unless compile-time gating is unavoidable.
- Can it be turned off without a code change if it misbehaves in dogfood/preview?
- Is the change reversible, or does it bake in a one-way decision (data format, public-ish API, user-visible contract) that's expensive to walk back?

### 5. Long-term maintainability
- Does it follow `WARP.md` Rust conventions: no needless type annotations, imports over long path qualifiers, `ctx` named and placed last, no `_`-prefixed unused params (remove them), inline format args, exhaustive matches over wildcard `_` arms?
- For WarpUI code: no inline `MouseStateHandle::default()` during render/event handling (construct once, clone/reference). Remember WarpUI is Metal-based, not GPUI/native — macOS look is *emulated*.
- Is the test strategy right for the risk? Don't demand tests that only vary struct fields; do flag a genuinely untested new code path or edge case. Integration-level UI/terminal behavior belongs in `warp-integration-test`.
- Naming and product terminology consistent across UI, comments, telemetry, and errors?

## Output format

Produce a single report. No preamble, no inline GitHub comments (that's `review-pr`'s job).

```
# Staff-architect review — <branch or PR#>

**Verdict:** <Ship · Ship with follow-ups · Reconsider design>
**Scope:** <one line: what the change does and its blast radius>

## Top risks
1. <highest-leverage concern — what breaks or what we regret, and why>
2. ...
(omit the section if genuinely none)

## By lens
- **Architecture:** <finding or "fits existing patterns">
- **Blast radius:** <finding or "contained">
- **Fork discipline:** <finding or "no gratuitous upstream divergence">
- **Rollout:** <flag/gating recommendation or "safe unconditionally">
- **Maintainability:** <finding or "conventions followed">

## Recommendations
- [ ] <concrete, actionable change — design-level, not a nit>
- [ ] ...

## Follow-ups (non-blocking)
- <smaller items worth a later PR; or "none">
```

Verdict rubric:
- **Ship** — design is sound; any notes are optional.
- **Ship with follow-ups** — mergeable, but named follow-ups should be tracked (flag, test, migration, or upstream-divergence cleanup).
- **Reconsider design** — a structural problem (wrong seam, irreversible mistake, large gratuitous fork divergence, unguarded risky rollout) that's cheaper to fix now than after merge. Lead with the alternative design, not just the objection.

## Rules

- **Altitude, not nits.** If your finding is a one-line fix, it belongs in `/code-review`. Keep this review about shape, risk, and cost.
- **Be specific and falsifiable.** "This increases coupling" is useless; "this makes `TerminalModel` reach into the view layer, so any view refactor now risks the model — move X behind the existing Y seam" is a review.
- **Recommend, don't just object.** Every "reconsider" must come with the alternative you'd take.
- **Read-only.** This skill reviews and reports; it does not edit code, push, comment on GitHub, or merge. If the user wants fixes applied, hand off to `/code-review --fix` or implement separately.
- **Respect the fork rules.** Never push, comment, or open issues/PRs against `warpdotdev/warp`. PR lookups use `--repo timomak/twarp` (see `CLAUDE.md`).
- **Match effort to the change.** A one-file tweak gets a few lines; a new pane / subsystem / persisted format gets the full lens sweep.
