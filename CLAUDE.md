# twarp — Claude collaborator notes

## Git remotes

This repo has two remotes:

- `origin` → `timomak/twarp` (the fork; this is where PRs go)
- `upstream` → `warpdotdev/warp` (read-only — never push, comment, or open issues/PRs here without explicit user approval)

## `gh pr create` — always pin the repo and head

`gh pr create` defaults to whichever remote it picks first when both `origin` and `upstream` are configured. In this repo it has been observed to pick `upstream` (`warpdotdev/warp`) and silently open the PR there. That violates the upstream-is-read-only rule.

Always pass `--repo` and `--head` explicitly:

```
gh pr create \
  --repo timomak/twarp \
  --base master \
  --head timomak:<branch-name> \
  --title "..." \
  --body "..."
```

Same applies to `gh pr close`, `gh issue create`, `gh pr comment`, etc. — pass `--repo timomak/twarp` so the action lands on the fork. Only target `warpdotdev/warp` when the user has explicitly approved a specific upstream action.
