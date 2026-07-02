# 09 - Rebrand to twarp (TECH)

Implements [PRODUCT.md](PRODUCT.md). This is a planned multi-PR rename. All file references were checked against the tree when this spec was authored; line numbers are approximate and will drift. Re-run the audit before each implementation sub-phase.

## Context

The current tree is still structurally a Warp workspace with twarp-specific feature work layered on top. `STATUS.md` lists roughly 21,400 case-insensitive matches across roughly 2,540 files; that scale is why 9a produces an audit and every later phase consumes it.

Important current anchors:

- `Cargo.toml:1` - workspace includes `app` and `crates/*`; default members still include `crates/warpui`, `crates/warp_completer`, `crates/warp_terminal`, and `crates/warp_util`.
- `Cargo.toml:25` - workspace authors are `Warp Team <dev@warp.dev>`.
- `Cargo.toml:69` - workspace dependency keys include `warp`, `warp_cli`, `warp_completer`, `warp_core`, `warp_features`, `warp_files`, `warp_graphql`, `warp_graphql_schema`, `warp_isolation_platform`, `warp_js`, `warp_logging`, `warp_managed_secrets`, `warp_ripgrep`, `warp_server_client`, `warp_terminal`, `warp_util`, `warp_web_event_bus`, `warpui`, `warpui_core`, and `warpui_extras`.
- `app/Cargo.toml:1` - app package/lib is named `warp`, `default-run` is `warp-oss`, and bins include `warp-oss` plus `warp`.
- `app/Cargo.toml:941` - cargo-bundle metadata defines channel identifiers such as `dev.warp.WarpOss`, `dev.warp.Warp-Stable`, `dev.warp.Warp-Preview`, `dev.warp.Warp-Dev`, and `dev.warp.Warp-Local`.
- `app/src/bin/oss.rs:23` - OSS build already has twarp-specific feature flags, but `AppId::new("dev", "warp", "WarpOss")`, `logfile_name: "warp-oss.log"`, and embedded plist values still use Warp names.
- `app/src/bin/local.rs:37` - local embedded plist uses `WarpLocal`, executable `warp`, bundle ID `dev.warp.Warp-Local`, and URL scheme `warplocal`.
- `app/DockTilePlugin/Info.plist:5`, `app/DockTilePlugin/Makefile:1`, and `app/DockTilePlugin/WarpDockTilePlugin.m:1` - DockTilePlugin bundle/executable/source names are Warp-branded and are loaded by `app/src/appearance.rs:228`.
- `app/src/uri/mod.rs:59` and `app/src/uri/uri_tests.rs:505` - desktop URI parsing and tests use `warp://`; related comments exist in `app/src/linear.rs:4`, `app/src/settings_view/mcp_servers_page.rs:32`, and `app/src/terminal/view.rs:12449`.
- `app/src/app_services/linux/mod.rs:156` - Linux app service defaults to `dev.warp.WarpLocal` and `/dev/warp/WarpLocal`.
- `crates/warp_core/src/paths.rs:220` - app data/cache/state directories derive from `ChannelState::app_id()`; macOS secure state joins the bundle/project path, and Linux maps `Warp` names to `warp-terminal` style directories.
- `crates/warp_core/src/paths.rs:278` - macOS app group ID is built as `{}.dev.warp`.
- `crates/warp_core/src/channel/config.rs:31` - `WarpServerConfig::production()` still points at `https://app.warp.dev`, `wss://rtc.app.warp.dev/graphql/v2`, and `wss://sessions.app.warp.dev`; `OzConfig::production()` points at `https://oz.warp.dev`.
- `app/src/auth/auth_view_shared_helpers.rs:127`, `app/src/settings_view/about_page.rs:70`, `app/src/pane_group/pane/get_started_view.rs:222`, and `app/src/pane_group/pane/welcome_view.rs:246` - visible logo surfaces reference Warp-branded SVG assets.
- `app/src/drive/index.rs:175`, `app/src/search/command_palette/warp_drive/data_source.rs`, and many `OpenWarpDrive*` symbols show the main internal/product fragment cluster for 9i.
- `.warp/`, `.warpindexingignore`, and README references to `WARP.md` are tracked repo hygiene inputs and must move in 9i unless the audit marks compatibility aliases.
- `.github/workflows/create_release.yml` and `script/bundle` are the release/package orchestration points; release workflows currently publish to `warp-releases/...` and copy CLI artifacts to `oz-*` names in some jobs.

The work must preserve AGPL/MIT/license attribution and must not push, comment, or open PRs against `upstream`.

## Proposed changes

### Global rule: audit first, then consume it

9a creates `roadmap/09-rebrand/AUDIT.md` and is the only sub-phase that should try to classify the whole repository in one pass. Later workers should edit only their sub-phase's audited file list plus any new compile fallout directly caused by those edits.

Recommended audit shape:

```markdown
# 09 rebrand audit

## Summary
- Generated: <date/commit>
- Search commands: ...
- Counts by classification: rename / replace / keep / regenerate

## 9b - Workspace + crate renames
| File/pattern | Current | Target | Class | Notes |
...
```

Use `git grep` because this node lacks `rg`. Suggested searches:

```sh
git grep -n -I -E '\bWarp\b|\bwarp\b|\bWARP\b|warp_|warp-|warpui|dev\.warp|warp://|\.warpindexingignore|WARP\.md'
git ls-files | grep -E '(^|/)(.*[Ww]arp.*|\.warp.*|WARP\.md)$'
```

Binary assets should be listed by filename/reference and visual inspection rather than text grep.

### 9a - Audit doc

Produce only `AUDIT.md`. Include:

- A top-level inventory for every match and file-name hit.
- Classification: `rename` for mechanical symbol/file moves; `replace` for user copy and URLs that need wording decisions; `regenerate` for icons/bitmap assets; `keep` for legal, provenance, historical comments, third-party URLs, protocol compatibility, and migration identifiers.
- An explicit "What stays Warp" section aligned with `STATUS.md`: licenses, README fork attribution, `warpdotdev/warp@d0f045c0`, copyright notices, and upstream-context comments.
- A "dangerous compatibility names" section for `WARP_*` shell variables, OSC hook names, telemetry event schema names, DB migration directory names, and old `warp://` compatibility.

### 9b - Workspace and crate renames

Rename workspace package/lib/dependency identifiers and directories where appropriate:

- `app` package/lib `warp` -> `twarp`.
- `crates/warp_cli` -> `crates/twarp_cli`; same for `warp_completer`, `warp_core`, `warp_features`, `warp_files`, `warp_graphql_schema`, `warp_js`, `warp_logging`, `warp_managed_secrets`, `warp_ripgrep`, `warp_server_client`, `warp_terminal`, `warp_util`, `warp_web_event_bus`.
- Crates stored in generic directories but named with Warp also change package names: `crates/editor` package `warp_editor` -> `twarp_editor`, `crates/graphql` package `warp_graphql` -> `twarp_graphql`, `crates/isolation_platform` package `warp_isolation_platform` -> `twarp_isolation_platform`.
- `crates/warpui`, `crates/warpui_core`, `crates/warpui_extras` -> `crates/twarpui`, `crates/twarpui_core`, `crates/twarpui_extras`.

Use `cargo metadata` after each mechanical batch to find stale dependency keys. Prefer atomic mechanical renames with `git mv` during implementation so history survives. Update proc-macro imports, generated module references, tests, CI package selectors, and comments only when they are product-branded rather than historical.

Do not change binary names here unless required to keep the workspace compiling; 9c owns executable names.

### 9c - CLI binary and URI scheme

Update binary targets and deep-link scheme:

- `app/Cargo.toml`: `default-run = "twarp-oss"`, `[[bin]] name = "twarp-oss"`, local bin `name = "twarp"`, and bundle metadata tables keyed by the new bin names.
- `app/src/bin/oss.rs` and `app/src/bin/local.rs`: embedded plist executable names, app IDs, log filenames, and display names.
- URI parsing/tests under `app/src/uri`, `app/src/linear.rs`, `app/src/settings_view/mcp_servers_page.rs`, `app/src/workspace/view/cloud_agent_capacity_modal/mod.rs`, and any command-palette/settings link constants.
- Shell/CLI-agent sentinel comments and tests around `warp://cli-agent` in `app/src/terminal/view.rs`.

Compatibility choice: `twarp://` is canonical. If old `warp://` parsing is retained, implement it as a migration alias in parser code and test both schemes, but keep UI/help examples on `twarp://`.

### 9d - Bundle IDs and native plists

Adopt `dev.twarp.*` throughout channel identity:

- `app/Cargo.toml` bundle metadata: `dev.twarp.TwarpOss`, `dev.twarp.Twarp-Stable`, `dev.twarp.Twarp-Preview`, `dev.twarp.Twarp-Dev`, `dev.twarp.Twarp-Local`; names `TwarpOss`, `Twarp`, `TwarpPreview`, `TwarpDev`, `TwarpLocal`.
- Embedded plist strings in `app/src/bin/*.rs` and any external plist generation paths.
- `crates/twarp_core/src/app_id.rs`, channel config defaults, and path tests after 9b.
- `crates/twarp_core/src/paths.rs`: Linux directory mapping should produce twarp package/data names, and macOS app group should use `{}.dev.twarp`.
- `app/src/app_services/linux/mod.rs`: D-Bus service/path defaults.
- Windows single-instance/login-item/registry strings in `app/src/app_services/windows/*` and `app/src/login_item/windows.rs`.
- DockTilePlugin plist `MainAppBundleIdentifier` flow after 9e renames the plugin.

Persisted data migration must be explicit. The conservative implementation is to start Twarp with new `dev.twarp.*` data roots while leaving old `dev.warp.*` data untouched; if migration is implemented, copy/migrate rather than move/delete and gate it with tests.

### 9e - Brand assets

Replace or regenerate visible Warp assets:

- Logo SVGs referenced from auth/about/welcome/get-started surfaces, including `warp-logo-light.svg`, `warp-logo-dark.svg`, `warp-logo-neutral.svg`, and `warp-logo-with-*-title.svg`.
- Channel icons under cargo-bundle metadata and `app/assets` channel resources.
- DockTilePlugin resources under `app/DockTilePlugin/Resources`.
- README badges that are product branding, such as "Built with Warp", should become twarp-owned assets or be removed; upstream attribution can remain text.

Rename DockTilePlugin source and bundle:

- `WarpDockTilePlugin.{m,h}` -> `TwarpDockTilePlugin.{m,h}`.
- Objective-C class `WarpDockTilePlugIn` -> `TwarpDockTilePlugIn`.
- Bundle/executable `WarpDockTilePlugin.docktileplugin` -> `TwarpDockTilePlugin.docktileplugin`.
- Update `app/src/appearance.rs` plugin lookup and `app/DockTilePlugin/Makefile`.

For new raster assets, prefer checked-in generated assets with deterministic names. Do not depend on network generation during build.

### 9f - User-facing strings

Consume the audit's `replace` list for visible text:

- Menus: `app/src/app_menus.rs`.
- Auth/onboarding/privacy/about surfaces: `app/src/auth/*`, `app/src/settings_view/about_page.rs`, onboarding assets/copy.
- Settings and feature pages under `app/src/settings_view`.
- Drive/search/resource-center labels under `app/src/drive`, `app/src/search`, and `app/src/resource_center`.
- Error messages, tooltips, accessibility/help labels, and docs links.

Keep comments and telemetry schema descriptions only if the audit marks them as historical/compatibility. Do not revive hidden or deleted AI/account/billing UI just to rename strings.

### 9g - Build scripts and installers

Update bundle, package, installer, and workflow naming:

- `script/bundle`, `script/mac/*`, `script/linux/*`, `script/windows/*`, `script/wasm/*`, and helper scripts that derive executable/package/artifact names.
- Linux `.desktop` generation, AppImage metadata, deb/rpm/Arch package metadata, package signing/upload scripts, and `.github/actions/bundle_arch_package`.
- Windows Inno Setup (`.iss`) generation and release workflow calls around `script/bundle -Channel ...`.
- `.github/workflows/create_release.yml`: artifact names and upload destinations currently include `warp-releases/...` and should move to twarp-owned destinations or local artifacts.
- CI references to old package names after 9b/9c.

Keep branch/workflow names that are purely historical only if the audit says `keep`; otherwise rename workflow display names and artifact labels to Twarp.

### 9h - Servers and telemetry

Make upstream network usage explicit:

- In channel config after 9b (`crates/twarp_core/src/channel/config.rs`), add a twarp/local disabled config or replace `WarpServerConfig::production()` usage in OSS/local binaries so default twarp builds do not point at `app.warp.dev`.
- `OzConfig::production()` should not point at `oz.warp.dev` for twarp unless explicitly retained as an upstream docs/provenance URL; feature 02 removed the AI surface, so prefer disabled/no-op config for shipped builds.
- Keep `telemetry_config: None`, `crash_reporting_config: None`, and `autoupdate_config: None` for OSS/local unless future owner configuration exists.
- Audit release-channel configs loaded by `app/src/bin/channel_config.rs` and `.github/workflows/release_configurations.json`; avoid producing release bundles that send telemetry/crashes to Warp-owned RudderStack/Sentry or download updates from Warp buckets.
- Update privacy copy and telemetry filenames if any visible/active telemetry surface remains.

The safe default for auto-update is disabled. A future self-hosted endpoint can re-enable it with its own spec.

### 9i - Internal feature names and local files

Rename product-fragment identifiers:

- `warp_drive` modules/paths -> `twarp_drive` where they are not public compatibility paths; `OpenWarpDrive*` -> `OpenTwarpDrive*`; visible "Warp Drive" -> "Twarp Drive".
- `warpify`/`Warpify` -> `twarpify`/`Twarpify` for user-facing SSH/session branding and internal module names if protocol compatibility permits.
- `warp_pack` -> `twarp_pack`; `open_in_warp` -> `open_in_twarp`.
- `.warp/` -> `.twarp/`, `.warpindexingignore` -> `.twarpindexingignore`, `WARP.md` -> `TWARP.md`; update contributing/readme/workflow references.

Compatibility exceptions:

- `WARP_*` shell environment variables and OSC hook names may be protocol names. Rename only with a compatibility bridge if existing shell bootstrap, SSH, and integration tests continue to pass.
- Diesel migration directory names or schema rows that include Warp fragments should stay if changing them risks rerunning or orphaning migrations. Prefer renaming surrounding Rust symbols and user strings while marking migration filenames `keep`.
- Telemetry event enum variant names and serialized event names may be historical schema. If telemetry remains disabled, do not churn schema solely for aesthetics unless the audit assigns that work to 9i.

### 9j - Cleanup sweep

Run final searches and close the audit:

- Whole-repo text grep for `Warp`, `warp`, `WARP`, `warp_`, `warp-`, `warpui`, `dev.warp`, `warp://`, `.warp`, `.warpindexingignore`, `WARP.md`.
- File-name grep over `git ls-files`.
- Binary/asset checklist from 9e.
- `cargo metadata`, `cargo fmt -- --check`, `cargo build --bin twarp-oss`, and `cargo clippy --workspace -- -D warnings`.

Every remaining match must be listed in `AUDIT.md` as `keep` with a reason. The final PR should not rely on "grep has too many false positives" as the explanation.

## Sub-phase breakdown

| Sub-phase | Owns | Primary validation |
| --- | --- | --- |
| 9a | `AUDIT.md` only | Product smoke 1-2 |
| 9b | Cargo package/crate/dependency renames | `cargo metadata`, build |
| 9c | Binary names and URI scheme | `cargo build --bin twarp-oss`, URI tests |
| 9d | Bundle IDs, plists, app IDs, OS service names, data path policy | plist/path tests, manual launch |
| 9e | Logo/icon/channel assets and DockTilePlugin rename | asset references, macOS icon smoke |
| 9f | User-facing copy | manual UI sweep, text grep |
| 9g | Build scripts, package metadata, installers, workflows | bundle check, artifact inspection |
| 9h | Server URLs, telemetry, crash, auto-update behavior | config inspection, network-disabled smoke |
| 9i | Internal product-fragment names and `.twarp` repo/runtime files | targeted tests for shell/bootstrap/indexing/migrations |
| 9j | Final cleanup and audit closure | final grep, fmt/build/clippy |

## Parallelization

The implementation should remain one sub-phase per branch, matching `AGENTS.md`, because 9b and 9c create extreme merge churn. Parallel work is useful only after 9a lands and only when the dispatcher can merge in a controlled order.

Recommended sequencing:

1. 9a first, alone.
2. 9b then 9c, sequentially. These invalidate imports, binary names, and many script references.
3. 9d and 9e can be prepared in parallel worktrees after 9c, but the DockTilePlugin touch point crosses both; coordinate by letting 9e own plugin source/assets and 9d own plist/app IDs.
4. 9f and 9i can start from the post-9c branch using the audit, but expect conflicts in Drive/warpify strings and symbols.
5. 9g should start after 9b/9c and preferably after 9d so package metadata has final channel names.
6. 9h can run after 9c; it touches config/network surfaces more than filesystem renames.
7. 9j must be last.

Suggested branch/worktree names for fleet dispatch are `twarp-09a-rebrand-audit`, `twarp-09b-crate-renames`, ..., `twarp-09j-rebrand-cleanup`. Each branch should target `origin/master`, never `upstream`, and should not open upstream PRs.

## Risks and mitigations

- **Merge churn:** crate and file renames will conflict with nearly every upstream cherry-pick. Mitigation: land 9b/9c only after an upstream catch-up and keep later sub-phases small.
- **Persisted data loss:** bundle ID/path changes can strand or delete existing local data. Mitigation: leave old data untouched unless an explicit copy migration is implemented and tested.
- **Protocol breakage:** `WARP_*` shell variables and OSC hook names may be protocol names rather than branding. Mitigation: classify them in 9a and keep or bridge them.
- **Telemetry/schema churn:** historical telemetry names may be serialized API contracts. Mitigation: because OSS/local telemetry is disabled, avoid cosmetic schema renames unless needed for visible product behavior.
- **Asset gaps:** missing icons can break bundle generation late. Mitigation: 9e must verify every asset reference and channel icon output before 9g.
- **Upstream service leakage:** production configs currently point at Warp services. Mitigation: 9h makes OSS/local no-op or twarp-owned by default and verifies with config/network inspection.

## Testing and validation

Use the PRODUCT smoke steps as the manual acceptance gate. Automated validation should be added where the surface is testable:

- 9b: `cargo metadata --format-version=1` plus `cargo build`; use package-name grep to catch old local crate keys.
- 9c: URI parser unit tests for `twarp://`; if `warp://` compatibility remains, include explicit compatibility tests.
- 9d: update `crates/twarp_core/src/paths_tests.rs` after 9b and app ID tests for `dev.twarp.*`; inspect embedded plists.
- 9e: asset reference tests if available; otherwise bundle/icon generation plus manual visual inspection.
- 9f: text grep over app UI source plus manual UI sweep.
- 9g: platform bundle check commands available on the worker; inspect generated metadata even when full signing/release publishing is unavailable.
- 9h: unit tests for disabled/twarp-owned channel config and manual/network-log confirmation that OSS/local does not contact upstream Warp endpoints.
- 9i: targeted tests for shell bootstrap/SSH variables, codebase ignore-file discovery, and migration behavior.
- 9j: final `cargo fmt -- --check`, `cargo build --bin twarp-oss`, `cargo clippy --workspace -- -D warnings`, and closed audit grep.
