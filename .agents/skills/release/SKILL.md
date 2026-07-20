---
name: release
description: Cut a twarp release from master — build the release bundle, install it locally WITHOUT launching, zip it, and publish a GitHub release on timomak/twarp. Use when the user runs /release or asks to make/cut/publish a new release.
---

# Release twarp

Cut a tagged GitHub release from `master` and install the new build locally.
The app must NOT be launched at any point — install only.

## Preconditions

1. Everything that should ship is **merged to master** (this skill does not
   merge PRs). `git fetch origin && git log origin/master -1` to confirm the
   expected commit is there.
2. If the main checkout is dirty with unrelated WIP (fleet/Codex often leave
   some), build from a **clean worktree** of `origin/master` instead of the
   main checkout: `git worktree add /tmp/twarp-release origin/master`.
   Remove it afterwards (`git worktree remove /tmp/twarp-release`).

## Procedure

All commands from the repo root being built (main checkout or the worktree).

1. **Pick the tag**: `vYYYY.MM.DD` from today's date. If that tag already
   exists (`git tag -l`/`gh release list --repo timomak/twarp`), use
   `vYYYY.MM.DD.N` with the next free N (same-day releases).
2. **Build + install** (this installs the bundle into /Applications but must
   not open it):

   ```
   ./script/run --release --install --dont-open
   ```

   `--dont-open` matters: without it the script launches the freshly built
   binary. Do NOT run `open` on the app afterwards either — the owner
   launches it themselves.
   If a signing/keychain error appears, see the keychain-signing memory
   (WARP_SIGNING_TEAM pin) before retrying.
3. **Zip the bundle** (stage in /tmp so the zip never lands in the repo):

   ```
   ditto -c -k --keepParent target/release/bundle/osx/Twarp.app /tmp/Twarp-<tag>.zip
   ```

   Zip the bundle that `./script/run --release` produced and keep the public
   artifact name aligned with the app name.
4. **Publish** — always pin the repo; never target warpdotdev/warp:

   ```
   gh release create <tag> /tmp/Twarp-<tag>.zip \
     --repo timomak/twarp --target master \
     --title "<tag>" --notes "<short changelog from git log since the previous tag>"
   ```

5. **Report**: release URL, zip size, installed app path, and a reminder that
   the app was installed but not launched.
