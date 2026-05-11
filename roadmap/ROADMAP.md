# twarp roadmap

Single source of truth for what's being built next. `/twarp-next` reads this file every invocation; the user reads it to see status at a glance.

**Currently active:** `03-md-rendered`

## Features

| # | Feature | Phase | Spec PR | Impl PR(s) |
|---|---------|-------|---------|-----------|
| 01 | [Tab color shortcuts](01-tab-colors/STATUS.md) | merged | [#2](https://github.com/timomak/twarp/pull/2) | [#3](https://github.com/timomak/twarp/pull/3) |
| 02 | [AI removal](02-ai-removal/STATUS.md) | merged | [#4](https://github.com/timomak/twarp/pull/4) | [#6](https://github.com/timomak/twarp/pull/6), [#7](https://github.com/timomak/twarp/pull/7), [#9](https://github.com/timomak/twarp/pull/9), [#10](https://github.com/timomak/twarp/pull/10), [#11](https://github.com/timomak/twarp/pull/11), [#12](https://github.com/timomak/twarp/pull/12), [#13](https://github.com/timomak/twarp/pull/13), [#14](https://github.com/timomak/twarp/pull/14), [#15](https://github.com/timomak/twarp/pull/15), [#16](https://github.com/timomak/twarp/pull/16), [#17](https://github.com/timomak/twarp/pull/17), [#18](https://github.com/timomak/twarp/pull/18) |
| 03 | [Render markdown by default](03-md-rendered/STATUS.md) | impl-in-review | [#49](https://github.com/timomak/twarp/pull/49) | [#50](https://github.com/timomak/twarp/pull/50) |
| 04 | [Custom command shortcuts](04-command-shortcuts/STATUS.md) | not-started | — | — |
| 05 | [Open Changes panel](05-open-changes/STATUS.md) | not-started | — | — |
| 06 | [Tab rename shortcut](06-tab-rename/STATUS.md) | not-started | — | — |
| 07 | [Rebrand to twarp](07-rebrand/STATUS.md) | not-started | — | — |

## Phases

- `not-started` — no work begun
- `spec-pending` — `/twarp-next` is writing PRODUCT.md / TECH.md
- `spec-in-review` — spec PR open, awaiting user review + merge
- `impl-pending` — specs merged, `/twarp-next` is implementing the next sub-phase
- `impl-in-review` — impl PR open, awaiting user review + merge
- `merged` — feature shipped

## Rules

- Only one feature is active at a time.
- A feature advances from `spec-in-review` → `impl-pending` only after the spec PR is **merged to master**.
- Features 02, 05, and 07 are sub-phased; their STATUS.md tracks individual sub-PRs and the feature only reaches `merged` after every sub-PR ships.
- The next feature only starts after the current one reaches `merged`.
- Git is the source of truth. If STATUS.md and `gh pr view` disagree, trust git and update STATUS.md.

## Order rationale

1. **Tab colors first** — smallest scope, validates the workflow at low risk; upstream has groundwork on `oz-agent/APP-4321-active-tab-color-indication`.
2. **AI removal second** — establishes the fork's identity. Cherry-pick conflicts from upstream become unavoidable from here, so eat the cost after the workflow is proven.
3. **Render markdown by default third** — small default flip on whatever surface(s) twarp uses to display `.md` files. After AI removal so the markdown render path isn't entangled with the deleted assistant transcript renderer.
4. **Command shortcuts fourth** — independent subsystem, no dependency on 01–03.
5. **Open Changes panel fifth** — largest user-facing scope, sub-phased into panel scaffold → diffs → staging → commit/push → file timeline.
6. **Tab rename shortcut sixth** — small, isolated keyboard binding that hooks into the existing rename interaction. Sequenced here only because 03–05 were already queued; nothing about its scope blocks earlier placement, and it stays before rebrand so the rename keybinding lands in `twarp_*` crates rather than churning during 7b.
7. **Rebrand last** — file/crate renames are the worst case for git merges, so push them as late as possible to keep upstream cherry-picks clean. By feature 07, AI code is gone, so the brand surface to rename is smaller.

## Out of scope for `/twarp-next`

- **Upstream cherry-picks.** Run on a separate cadence — schedule a recurring agent (`/schedule`) to fetch, list new commits, and propose cherry-picks. Not driven by this skill.
- **CI / repo hygiene unrelated to the active feature.**

## Spec storage convention

For twarp roadmap features, specs live alongside `STATUS.md`:

```
roadmap/<NN-feature>/PRODUCT.md
roadmap/<NN-feature>/TECH.md
```

This intentionally overrides the repo's default `specs/<linear-ticket>/...` convention, because twarp roadmap features are not tracked in Linear.
