# twarp roadmap

Single source of truth for what's being built next. `/twarp-next` reads this file every invocation; the user reads it to see status at a glance.

**Currently active:** `05-open-changes`

## Features

| # | Feature | Phase | Spec PR | Impl PR(s) |
|---|---------|-------|---------|-----------|
| 01 | [Tab color shortcuts](01-tab-colors/STATUS.md) | merged | [#2](https://github.com/timomak/twarp/pull/2) | [#3](https://github.com/timomak/twarp/pull/3) |
| 02 | [AI removal](02-ai-removal/STATUS.md) | merged | [#4](https://github.com/timomak/twarp/pull/4) | [#6](https://github.com/timomak/twarp/pull/6), [#7](https://github.com/timomak/twarp/pull/7), [#9](https://github.com/timomak/twarp/pull/9), [#10](https://github.com/timomak/twarp/pull/10), [#11](https://github.com/timomak/twarp/pull/11), [#12](https://github.com/timomak/twarp/pull/12), [#13](https://github.com/timomak/twarp/pull/13), [#14](https://github.com/timomak/twarp/pull/14), [#15](https://github.com/timomak/twarp/pull/15), [#16](https://github.com/timomak/twarp/pull/16), [#17](https://github.com/timomak/twarp/pull/17), [#18](https://github.com/timomak/twarp/pull/18) |
| 03 | [Render markdown by default](03-md-rendered/STATUS.md) | merged | [#49](https://github.com/timomak/twarp/pull/49) | [#50](https://github.com/timomak/twarp/pull/50) |
| 04 | [Custom command shortcuts](04-command-shortcuts/STATUS.md) | merged | [#51](https://github.com/timomak/twarp/pull/51) | 4a [#52](https://github.com/timomak/twarp/pull/52), 4b [#53](https://github.com/timomak/twarp/pull/53), 4c [#54](https://github.com/timomak/twarp/pull/54), 4d [#55](https://github.com/timomak/twarp/pull/55) |
| 05 | [Open Changes panel](05-open-changes/STATUS.md) | impl-in-review | [#56](https://github.com/timomak/twarp/pull/56), respec [#58](https://github.com/timomak/twarp/pull/58) | 5a [#59](https://github.com/timomak/twarp/pull/59), 5c+5e [#60](https://github.com/timomak/twarp/pull/60), 5e polish [#61](https://github.com/timomak/twarp/pull/61), 5b [#62](https://github.com/timomak/twarp/pull/62) |
| 06 | [Tab rename shortcut](06-tab-rename/STATUS.md) | not-started | — | — |
| 07 | [Claude Code panel](07-claude-code-panel/STATUS.md) | not-started | — | — |
| 08 | [Rebrand to twarp](08-rebrand/STATUS.md) | not-started | — | — |
| 09 | [File editor with go-to-definition](09-file-editor/STATUS.md) | not-started | — | — |
| 10 | [Git blame](10-git-blame/STATUS.md) | not-started | — | — |
| 11 | [Project search & replace](11-project-search-replace/STATUS.md) | not-started | — | — |

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
- Features 02, 05, 07, 08, 09, 10, and 11 are sub-phased; their STATUS.md tracks individual sub-PRs and the feature only reaches `merged` after every sub-PR ships.
- The next feature only starts after the current one reaches `merged`.
- Git is the source of truth. If STATUS.md and `gh pr view` disagree, trust git and update STATUS.md.

## Order rationale

1. **Tab colors first** — smallest scope, validates the workflow at low risk; upstream has groundwork on `oz-agent/APP-4321-active-tab-color-indication`.
2. **AI removal second** — establishes the fork's identity. Cherry-pick conflicts from upstream become unavoidable from here, so eat the cost after the workflow is proven.
3. **Render markdown by default third** — small default flip on whatever surface(s) twarp uses to display `.md` files. After AI removal so the markdown render path isn't entangled with the deleted assistant transcript renderer.
4. **Command shortcuts fourth** — independent subsystem, no dependency on 01–03.
5. **Open Changes panel fifth** — largest user-facing scope, sub-phased into panel scaffold → diffs → staging → commit/push → file timeline.
6. **Tab rename shortcut sixth** — small, isolated keyboard binding that hooks into the existing rename interaction. Sequenced here only because 03–05 were already queued; nothing about its scope blocks earlier placement, and it stays before rebrand so the rename keybinding lands in `twarp_*` crates rather than churning during 8b.
7. **Claude Code panel seventh** — large user-facing scope, sub-phased. Re-introduces Warp Agent Mode's rendering layer (removed in feature 02) as a host for the local `claude` subprocess running on the user's Claude Max subscription. No LLM client, no billing, no cloud sync — only the renderer comes back. Slotted before the rebrand because cherry-picks from upstream agent crates are much harder once every `warp_*` / `warpui*` crate has been renamed.
8. **Rebrand last among the upstream-sensitive features** — file/crate renames are the worst case for git merges, so push them as late as possible to keep upstream cherry-picks clean. By feature 08, AI code is gone and the agent renderer is wired up, so the brand surface to rename is smaller.
9. **File editor surface ninth** — pivots twarp from "terminal" to "terminal + IDE" by exposing the existing `crates/editor/` + `crates/lsp/` infrastructure as a first-class file-editing workflow. Headline gesture is cmd+click → LSP definition (already callable from `app/src/code/local_code_editor.rs`, just not wired to a workflow where you can open arbitrary files). Placed after rebrand because wiring across `app/src/code/`, `crates/editor/`, and `crates/lsp/` would otherwise be churned during the rename pass.
10. **Git blame tenth** — depends on 09 (no blame without a file-editing surface). Genuinely net-new code: `git blame --porcelain` parser, gutter rendering, commit-detail popover. No upstream cherry-pick risk because blame is new.
11. **Project search & replace eleventh** — wires the existing `warp_ripgrep` crate into a project-wide search UI plus a replace-all flow. Independent of 09 in principle; sequenced after for result-click → open-file.

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
