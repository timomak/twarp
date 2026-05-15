# 10 — Git blame & per-line history

**Phase:** not-started
**Spec PR:** —
**Impl PRs:** —

## Scope

Render `git blame` information in the gutter of the file editor: per-line author, short commit hash, and relative date. Clicking the author or hash opens a popover with the commit's message and that commit's diff.

## Why this slot

Depends on 09 (no blame surface without a file-editing workflow). Genuinely net-new code — the existing decoration system can host gutter annotations, but the data source (`git blame --porcelain`) parser is fresh. No upstream cherry-pick risk because blame is new.

## Sub-phases

- [ ] **10a — Blame parser + gutter rendering.** Parse `git blame --porcelain` for the open file. Cache by file version. Render author / short hash / relative date in the editor gutter alongside line numbers and diff indicators.
- [ ] **10b — Commit detail popover.** Click on author or hash → popover showing commit message, author, full hash, date, and that commit's diff. Optionally link to a GitHub commit URL when origin is a GitHub remote.

## What's already built

- Decoration system (`crates/editor/src/decoration/`) supports gutter annotations alongside diagnostics
- Git command wrapper (`app/src/util/git.rs`) — straightforward to extend with `git blame --porcelain`
- Porcelain parsing pattern in `app/src/code_review/porcelain_v2.rs` as a template
- Hover / popover infrastructure for the commit detail UI
- Markdown rendering for commit messages

## Notes

- File version caching matters — re-running `git blame` on every keystroke would be expensive. Invalidate on save or external git operations (commit, checkout, rebase).
- Edited (uncommitted) lines should show "(uncommitted)" rather than stale blame.
- No upstream cherry-pick conflicts expected — this is net-new functionality.
