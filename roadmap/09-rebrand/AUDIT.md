# 09 rebrand audit

## Summary

- Generated: 2026-07-02
- Baseline commit: `c3c517b6`
- Scope: tracked files only.
- Text search count: `10,137` matching lines in `1,808` tracked text files.
- Filename/path search count: `765` tracked path hits.
- Combined tracked path inventory: `2,309` unique paths with either a text hit or a filename hit.

Run these exact commands from the repo root to reproduce the inventory:

```sh
git grep -n -I -E '\bWarp\b|\bwarp\b|\bWARP\b|warp_|warp-|warpui|dev\.warp|warp://|\.warpindexingignore|WARP\.md' -- . ':(exclude)roadmap/09-rebrand/AUDIT.md'
git grep -l -I -E '\bWarp\b|\bwarp\b|\bWARP\b|warp_|warp-|warpui|dev\.warp|warp://|\.warpindexingignore|WARP\.md' -- . ':(exclude)roadmap/09-rebrand/AUDIT.md'
git ls-files | grep -E '(^|/)(.*[Ww]arp.*|\.warp.*|WARP\.md)$'
```

The first command is the canonical match inventory. The second lists every text file that has a match. The third lists every tracked file whose path is Warp-branded even when the file is binary or has no searchable text. The text commands exclude this audit file so the audit does not count itself.

Classification counts below count audit rows, not raw grep lines, because one file can legitimately be owned by multiple sub-phases:

| Class | Audit rows | Meaning |
| --- | ---: | --- |
| `rename` | 37 | Mechanical symbol, crate, path, bundle ID, command, feature, or filename move. |
| `replace` | 25 | User copy, docs URL, endpoint, package metadata, or wording that needs a product decision. |
| `regenerate` | 9 | Image/icon/logo assets that must be recreated or visually inspected. |
| `keep` | 14 | Legal attribution, provenance, protocol compatibility, historical comments, serialized schemas, or migration identity. |

## Inventory ownership rules

Every match from the canonical commands is assigned by the first applicable rule in this table. If a file has more than one kind of match, it appears in multiple sub-phase sections below.

| Rule | Path or identifier pattern | Owner | Class | Reason |
| --- | --- | --- | --- | --- |
| Crate graph | `Cargo.toml`, `Cargo.lock`, `app/Cargo.toml`, `crates/**/Cargo.toml`, `crates/**/build.rs`, Rust imports of `warp_*` or `warpui*` | 9b | `rename` | Workspace package, lib, dependency, feature, and import names are mechanical crate graph moves. |
| Binary names | `app/Cargo.toml` bin/default-run entries, `app/src/bin/{oss,local}.rs`, command examples using `warp` or `warp-oss` | 9c | `rename` | The canonical executables become `twarp` and `twarp-oss`. |
| URI scheme | `warp://`, `warplocal://`, URI parser/tests, deep-link comments/constants | 9c | `replace` | The canonical scheme becomes `twarp://`; any old scheme support must be explicit migration compatibility. |
| Native identity | `dev.warp.*`, `WarpOss`, `WarpLocal`, `WarpPreview`, `WarpDev`, D-Bus paths, app group IDs, Windows instance IDs | 9d | `rename` | Bundle IDs and OS service identities become `dev.twarp.*` and `Twarp*`. |
| Data paths | `warp-terminal*`, `warp\\Warp*`, `dev.warp.Warp*`, app support/cache/state tests | 9d | `rename` | Persisted path policy belongs with bundle ID changes. Old directories must not be deleted. |
| App logos/icons | `app/assets/**/warp*.svg`, `app/assets/**/warp*.png`, `app/channels/**/icon/**`, `images/Built-With-Warp*`, `script/warp.svg`, Windows installer bitmaps | 9e | `regenerate` | Visible product assets need first-party Twarp replacements and reference updates. |
| DockTilePlugin assets/source | `app/DockTilePlugin/**`, `WarpDockTilePlugin*`, plugin resource names | 9e | `rename` | The plugin bundle, source files, Objective-C class, executable, and resources move to Twarp names. |
| Visible product copy | UI strings, menus, settings, about/onboarding, command palette, help/error text, docs links | 9f | `replace` | User-facing copy must say Twarp/twarp except explicit fork attribution. |
| Build/package scripts | `script/**`, `.github/workflows/**`, `.github/actions/**`, desktop/package/install metadata | 9g | `rename` | Generated artifact names, package IDs, release paths, and installer labels move to Twarp. |
| Release destinations | `warp-releases`, Warp release buckets, upload filenames, channel config JSON | 9g | `replace` | Twarp builds must not publish under Warp names or upstream buckets by default. |
| Network services | `app.warp.dev`, `rtc.app.warp.dev`, `sessions.app.warp.dev`, `oz.warp.dev`, telemetry/crash/autoupdate destinations | 9h | `replace` | OSS/local must not contact upstream Warp production services by accident. |
| Telemetry schemas | serialized event names, telemetry filenames, schema labels containing Warp fragments | 9h | `keep` | Telemetry is disabled for OSS/local; avoid schema churn unless a visible/product surface remains. |
| Internal feature fragments | `warp_drive`, `WarpDrive*`, `warpify`, `Warpify`, `warp_pack`, `OpenWarpDrive*`, `open_in_warp`, `Open in Warp` | 9i | `rename` | Internal product-branded symbols and still-visible feature labels move to Twarp equivalents where safe. |
| Repo/runtime hygiene | `.warp/`, `.warpindexingignore`, `WARP.md`, `~/.warp`, `WARP_*` repo-workflow env docs | 9i | `rename` | User-visible repo/runtime names move to `.twarp`, `.twarpindexingignore`, and `TWARP.md`; compatibility aliases must be deliberate. |
| Protocol compatibility | shell integration `WARP_*`, OSC hook comments/names, bootstrap sentinels | 9i | `keep` | These are protocol/API names until 9i implements and tests a compatibility bridge. |
| DB migrations | `crates/persistence/migrations/*warp_drive*`, `*warp_pack*`, persisted migration/schema IDs | 9i | `keep` | Renaming applied migration identities risks reruns or orphaned state. Rename surrounding Rust/UI names only. |
| Legal/provenance | `LICENSE-*`, copyright notices, README fork attribution, `warpdotdev/warp@d0f045c0`, upstream-context comments, third-party URLs | 9j | `keep` | Required attribution and historical context must remain explicit. |
| Roadmap/spec history | `roadmap/**`, `specs/**`, prior status/spec files | 9j | `keep` | Historical planning documents may reference Warp for provenance; final cleanup should classify remaining product branding. |
| Residual grep hits | Any canonical command match not covered above | 9j | `replace` | The cleanup sweep must either update it or add an explicit `keep` reason. |

## 9b - Workspace and crate renames

| File/pattern | Current | Target | Class | Notes |
| --- | --- | --- | --- | --- |
| `Cargo.toml`, `Cargo.lock` | workspace package/dependency keys `warp`, `warp_*`, `warpui*` | `twarp`, `twarp_*`, `twarpui*` | `rename` | Includes workspace members, default members, dependency keys, features, and lockfile package names. Keep `warp_multi_agent_api` only if later audit decides the upstream proto API name is an external dependency. |
| `app/Cargo.toml` package/lib/deps | package/lib `warp`, deps `warp_*`, `warpui*` | package/lib `twarp`, deps `twarp_*`, `twarpui*` | `rename` | 9b owns crate names only; 9c owns bin names and URI scheme rows in the same file. |
| `crates/warp_cli/**` | `warp_cli` | `twarp_cli` | `rename` | Directory, package, crate imports, tests. |
| `crates/warp_completer/**` | `warp_completer` | `twarp_completer` | `rename` | Directory, package, crate imports, parser/test references. |
| `crates/warp_core/**` | `warp_core` | `twarp_core` | `rename` | Directory and imports. 9d owns app ID/path strings inside the renamed crate. 9h owns channel server config strings. |
| `crates/warp_features/**` | `warp_features` | `twarp_features` | `rename` | Directory, package, feature import paths. |
| `crates/warp_files/**` | `warp_files` | `twarp_files` | `rename` | Directory, package, imports. |
| `crates/warp_graphql_schema/**` | `warp_graphql_schema` | `twarp_graphql_schema` | `rename` | Directory and generated schema crate imports. |
| `crates/warp_js/**` | `warp_js` | `twarp_js` | `rename` | Directory, optional dependency feature names. |
| `crates/warp_logging/**` | `warp_logging` | `twarp_logging` | `rename` | Directory, logging feature imports. |
| `crates/managed_secrets/**` | package `warp_managed_secrets` | package `twarp_managed_secrets` | `rename` | Directory can stay generic unless 9b chooses `git mv`; package/import names should move. |
| `crates/warp_ripgrep/**` | `warp_ripgrep` | `twarp_ripgrep` | `rename` | Directory, package, imports. |
| `crates/warp_server_client/**` | `warp_server_client` | `twarp_server_client` | `rename` | Directory and imports. 9h owns upstream service endpoints within this area. |
| `crates/warp_terminal/**` | `warp_terminal` | `twarp_terminal` | `rename` | Directory and imports. Preserve third-party terminal model license text. |
| `crates/warp_util/**` | `warp_util` | `twarp_util` | `rename` | Directory and imports. |
| `crates/warp_web_event_bus/**` | `warp_web_event_bus` | `twarp_web_event_bus` | `rename` | Directory and imports. |
| `crates/warpui/**`, `crates/warpui_core/**`, `crates/warpui_extras/**` | `warpui*` | `twarpui*` | `rename` | Directory/package/import names. Keep upstream-context comments only if still accurate. |
| `crates/editor/**`, `crates/graphql/**`, `crates/isolation_platform/**` | package names `warp_editor`, `warp_graphql`, `warp_isolation_platform` | `twarp_editor`, `twarp_graphql`, `twarp_isolation_platform` | `rename` | Generic directory names can stay; package/lib/dependency names move. |
| Other `crates/**` and `app/**` imports | `use warp_*`, `use warpui*`, dependency features such as `warpui/log_named_telemetry_events` | `use twarp_*`, `use twarpui*`, matching feature names | `rename` | Use the canonical grep plus `cargo metadata` to enumerate exact import fallout after directory moves. |

Out of scope for 9b: binary target names, bundle IDs, app data paths, service endpoints, visible UI copy, and generated assets.

## 9c - CLI binary and URI scheme

| File/pattern | Current | Target | Class | Notes |
| --- | --- | --- | --- | --- |
| `app/Cargo.toml` bin/default-run metadata | `default-run = "warp-oss"`, bin names `warp-oss` and `warp` | `twarp-oss` and `twarp` | `rename` | Keep crate-name edits in 9b separate from binary target edits here. |
| `app/src/bin/oss.rs` | `warp-oss` executable/plist/log comments | `twarp-oss` | `rename` | 9d owns `dev.warp.*` app ID values in the same file. |
| `app/src/bin/local.rs` | local executable `warp`, scheme `warplocal` | `twarp`, local scheme aligned to Twarp policy | `rename` | 9d owns display/bundle ID strings in the same file. |
| `app/src/uri/**` | parser/tests/examples using `warp://` and `warplocal://` | canonical `twarp://` | `replace` | If old `warp://` remains, it must be a tested migration alias and not appear in user-facing examples. |
| `app/src/linear.rs`, `app/src/settings_view/mcp_servers_page.rs`, `app/src/workspace/view/cloud_agent_capacity_modal/mod.rs` | deep-link comments/constants using `warp://` | `twarp://` | `replace` | Comments and settings/billing links should use canonical scheme unless compatibility-only. |
| `app/src/terminal/view.rs` | `warp://cli-agent` sentinel comment/path | `twarp://cli-agent` canonical, optional `warp://` fallback | `replace` | Treat fallback as dangerous compatibility and test both if retained. |

Out of scope for 9c: crate imports, bundle IDs, installer package names, and UI copy that does not mention executable names or URI schemes.

## 9d - Bundle IDs and native plists

| File/pattern | Current | Target | Class | Notes |
| --- | --- | --- | --- | --- |
| `app/Cargo.toml` bundle metadata | `dev.warp.WarpOss`, `dev.warp.Warp-Stable`, `dev.warp.Warp-Preview`, `dev.warp.Warp-Dev`, `dev.warp.Warp-Local` | `dev.twarp.TwarpOss`, `dev.twarp.Twarp-Stable`, `dev.twarp.Twarp-Preview`, `dev.twarp.Twarp-Dev`, `dev.twarp.Twarp-Local` | `rename` | Channel display names become `TwarpOss`, `Twarp`, `TwarpPreview`, `TwarpDev`, `TwarpLocal`. |
| `app/src/bin/{oss,local}.rs` embedded plist/app IDs | `WarpOss`, `WarpLocal`, `dev.warp.*`, `warposs`, `warplocal` | Twarp names and `dev.twarp.*` | `rename` | URI scheme naming crosses 9c; bundle identity belongs here. |
| `app/channels/*/dev.warp.*.desktop` | desktop filenames, `Name=Warp*`, `StartupWMClass=dev.warp.*`, `Icon=dev.warp.*`, `Exec=warp-terminal*` | `dev.twarp.*`, `Twarp*`, twarp executable/package names | `rename` | Desktop IDs cross 9g package metadata; keep both phases coordinated. |
| `app/src/app_services/linux/mod.rs` | `dev.warp.WarpLocal`, `/dev/warp/WarpLocal`, D-Bus comments | `dev.twarp.TwarpLocal`, `/dev/twarp/TwarpLocal` | `rename` | D-Bus well-known name/path should agree with desktop identity. |
| `app/src/app_services/windows/**`, `app/src/login_item/windows.rs` | registry, AppUserModelID, single-instance/channel names `Warp*` | Twarp channel identities | `rename` | Keep behavior isolated across channels. |
| `crates/warp_core/src/app_id.rs`, `channel/state.rs`, `paths.rs`, tests | `AppId::new("dev","warp","Warp*")`, `dev.warp`, `warp-terminal*`, `warp\\Warp*` | `dev.twarp`, `Twarp*`, `twarp-terminal*`, `twarp\\Twarp*` | `rename` | After 9b this path becomes `crates/twarp_core/**`. Old data roots must be left untouched or explicitly copied. |
| `script/Entitlements.plist` | `2BBY89MBSN.dev.warp` | `2BBY89MBSN.dev.twarp` or owner-supplied team/domain value | `rename` | The team ID may need owner confirmation; do not infer an upstream entitlement. |
| `app/src/persistence/README.md` | sample DB path under `dev.warp.Warp-Local/warp.sqlite` | Twarp sample path | `replace` | Documentation only; actual data migration policy must be tested. |

Out of scope for 9d: DockTilePlugin source/assets except plist identity coordination, and release artifact publishing.

## 9e - Brand assets

| File/pattern | Current | Target | Class | Notes |
| --- | --- | --- | --- | --- |
| `app/assets/bundled/svg/warp-logo-*.svg`, `warp-logo-with-*-title.svg`, `warp.svg`, `warp-2.svg`, `warp-3.svg`, `warp-drive.svg`, `warp-loading-*.svg` | Warp logo/wordmark/loading/Drive assets | First-party Twarp SVG set | `regenerate` | Verify every reference from auth/about/welcome/get-started/onboarding surfaces. |
| `app/assets/async/png/onboarding/*warp*`, `openwarp_launch_banner.png` | Warp Drive/OpenWarp onboarding bitmaps | Twarp-branded onboarding bitmaps | `regenerate` | Keep dimensions and reference names consistent or update references. |
| `app/assets/resources/mac/warp_install_image.png` | Warp install image | Twarp install image | `regenerate` | Used by macOS packaging. |
| `app/channels/**/icon/**`, especially `warp-glyph 3.svg` | channel icon assets | Twarp channel icon assets | `regenerate` | Stable/local/dev/preview/oss must remain visually distinguishable. |
| `app/DockTilePlugin/Resources/**`, especially `warp_2.png` | Warp-branded selectable icons | Twarp selectable icons | `regenerate` | Preserve user-selectable icon behavior. |
| `app/DockTilePlugin/{WarpDockTilePlugin.h,WarpDockTilePlugin.m,Info.plist,Makefile,README.md}` | `WarpDockTilePlugin`, `WarpDockTilePlugIn` | `TwarpDockTilePlugin`, `TwarpDockTilePlugIn` | `rename` | 9e owns plugin source/class/bundle/resource lookup; 9d owns app bundle ID values feeding plugin metadata. |
| `images/Built-With-Warp-Export@2x.png` | Warp badge | remove, replace with Twarp badge, or keep only as upstream attribution | `regenerate` | Product branding must not reuse the Warp badge. |
| `script/warp.svg`, `script/windows/installer-images/warp-{banner,logo}.bmp` | installer logo assets | Twarp installer assets | `regenerate` | Asset files cross 9g installer scripts. |

Out of scope for 9e: arbitrary UI copy around assets and package metadata not needed to resolve asset references.

## 9f - User-facing strings

| File/pattern | Current | Target | Class | Notes |
| --- | --- | --- | --- | --- |
| `app/src/app_menus.rs`, `app/src/menu*.rs` | menu labels containing Warp | Twarp | `replace` | User-facing menu copy only; leave crate/import identifiers to 9b. |
| `app/src/settings_view/**` | settings page labels, platform/billing/privacy copy, `warp_drive_page.rs`, `warpify_page.rs` | Twarp copy | `replace` | Do not revive removed account/billing/AI surfaces from feature 02. |
| `app/src/settings/**`, `app/src/user_config/**` | settings docs/help/example copy | Twarp copy | `replace` | Persisted setting keys need compatibility review before renaming. |
| `app/src/auth/**`, `app/src/pane_group/pane/{get_started_view,welcome_view}.rs`, onboarding text | auth/onboarding welcome copy and logo labels | Twarp copy plus fork attribution where appropriate | `replace` | Auth/cloud surfaces must remain disabled/dark if feature 02 removed them. |
| `app/src/drive/**`, `app/src/cloud_object/**`, `app/src/search/**`, `app/src/resource_center/**` | "Warp Drive", "Warp workflows", docs/help links | Twarp equivalents or hidden/removed copy | `replace` | Internal symbol renames belong to 9i; visible labels belong here. |
| `app/src/terminal/**` | "Warp prompt", "Warpify", "Open in Warp", tooltips/errors/help | Twarp prompt/Twarpify/Open in Twarp | `replace` | Shell protocol variables may remain `WARP_*` if classified as compatibility under 9i. |
| `app/src/code/**`, `app/src/code_review/**`, `app/src/notebooks/**`, `app/src/env_vars/**`, `app/src/workspace/**`, `app/src/workflows/**` | visible labels, errors, telemetry-adjacent UI copy | Twarp copy | `replace` | Serialized telemetry names are 9h `keep`; visible strings are `replace`. |
| `.github/ISSUE_TEMPLATE/**`, `.github/PULL_REQUEST_TEMPLATE/**`, `FAQ.md`, `README.md`, `CONTRIBUTING.md`, `resources/bundled/skills/**` | user/developer-facing copy | Twarp copy, except explicit fork/upstream attribution | `replace` | README should keep "fork of Warp" provenance while replacing product instructions and command names. |
| `docs/specs/roadmap historical files` (`specs/**`, old `roadmap/**`) | historical Warp references | Usually keep, unless visible current docs | `keep` | Do not rewrite old feature specs for this rebrand unless 9j identifies current product docs there. |

Out of scope for 9f: crate names, binary names, bundle IDs, server endpoints, and migration/protocol identifiers.

## 9g - Build scripts and installers

| File/pattern | Current | Target | Class | Notes |
| --- | --- | --- | --- | --- |
| `script/linux/bundle*`, `script/linux/linuxdeploy-plugin-warp` | `warp-terminal*`, `Warp*`, `dev.warp.*`, `WARP_PACKAGE_NAME` | twarp package names, `Twarp*`, `dev.twarp.*`, new variable names where safe | `rename` | Linux package IDs, desktop launchers, AppImage/deb/rpm/Arch metadata must agree. |
| `script/macos/bundle`, `script/macos/run`, `script/update_plist`, `script/install_channel_config` | `Warp*.app`, `WARP_APP_NAME`, `WARP_BIN`, `WarpDockTilePlugin` | `Twarp*.app`, twarp variables where practical, Twarp plugin | `rename` | Variable names can remain only if purely internal and 9j classifies them; generated output must be Twarp. |
| `script/windows/bundle.ps1`, `script/windows/windows-installer.iss`, `script/windows/installer-images/*warp*` | `Warp*`, `warp-terminal-*`, `dev.warp.*`, installer images | Twarp package/app IDs and Twarp images | `rename` | Coordinate with 9e for bitmap regeneration. |
| `.github/workflows/create_release.yml`, `release_configurations.json`, release/cut/delete workflows | artifact names, channels, upload paths containing Warp/warp | twarp artifact names and destinations | `replace` | Must not publish to upstream Warp buckets or emit Warp-named artifacts by default. |
| `.github/actions/**`, `.github/workflows/ci.yml`, workflow/package helper files | old package/binary names in build commands | `twarp`, `twarp-oss`, package names from 9b/9c/9d | `rename` | Update CI selectors after crate and binary renames land. |
| `app/channels/*/*.desktop` | desktop/package metadata | Twarp desktop/package metadata | `rename` | Identity rows are also owned by 9d; package/install behavior is 9g. |

Out of scope for 9g: app runtime endpoint behavior and visual asset creation.

## 9h - Servers and telemetry

| File/pattern | Current | Target | Class | Notes |
| --- | --- | --- | --- | --- |
| `crates/warp_core/src/channel/config.rs` | `https://app.warp.dev`, `wss://rtc.app.warp.dev/graphql/v2`, `wss://sessions.app.warp.dev`, `https://oz.warp.dev` | disabled/no-op for OSS/local or twarp-owned endpoints | `replace` | After 9b this path becomes `crates/twarp_core/**`. Do not silently use upstream services. |
| `app/src/bin/channel_config.rs`, `.github/workflows/release_configurations.json` | release channel endpoint/config references | Twarp-owned or disabled configs | `replace` | Release bundles must not configure upstream telemetry/crash/update destinations by default. |
| `app/src/autoupdate/**` | update package names/endpoints and comments | disabled or twarp-owned update flow | `replace` | Auto-update should be disabled unless a Twarp endpoint exists. |
| `app/src/crash_reporting/**`, `app/src/server/telemetry/**`, `app/src/code/lsp_telemetry.rs`, telemetry tests | Warp telemetry/crash destinations, privacy copy, event descriptions | disabled/twarp-owned runtime behavior; Twarp-visible copy | `replace` | Serialized event names can remain `keep` if disabled and not user-visible. |
| `app/src/server/**`, `app/src/auth/**`, `app/src/billing/**`, `app/src/cloud_object/**`, `crates/warp_server_client/**` | auth/cloud/billing/Warp Drive service calls | disabled/local-only/twarp-owned behavior | `replace` | Feature 02 removals must remain dark. |
| `app/src/server/telemetry/events.rs`, `app/src/code_review/telemetry_event.rs`, notebooks telemetry schemas | persisted/serialized event names with Warp fragments | no rename unless needed for active user-visible behavior | `keep` | Cosmetic schema churn is risky and unnecessary while telemetry is disabled. |
| External dependency URLs such as `warpdotdev/warp-proto-apis` | upstream proto/source URL | keep unless vendored or replaced intentionally | `keep` | External dependency provenance is not product branding. |

Out of scope for 9h: visible UI string-only replacements that do not affect service behavior.

## 9i - Internal feature names and local files

| File/pattern | Current | Target | Class | Notes |
| --- | --- | --- | --- | --- |
| `app/src/search/command_palette/warp_drive/**`, `app/src/integration_testing/warp_drive/**`, `app/src/settings_view/warp_drive_page.rs` | `warp_drive`, `WarpDrive*`, `OpenWarpDrive*` | `twarp_drive`, `TwarpDrive*`, `OpenTwarpDrive*` | `rename` | Visible "Warp Drive" copy is 9f; internal symbols/modules are 9i. |
| `app/src/drive/**`, `app/src/app_state.rs`, related GraphQL update files | `warp_drive_*`, `WarpDrive*` symbols/fields | `twarp_drive_*`, `TwarpDrive*` where not persisted protocol | `rename` | Persisted DB columns or GraphQL schema names need compatibility review before renaming. |
| `app/src/terminal/ssh/warpify.rs`, `app/src/terminal/warpify/**`, `app/src/settings_view/warpify_page.rs`, `app/assets/bundled/ssh/**/warpify_*.sh` | `warpify`, `Warpify` | `twarpify`, `Twarpify` | `rename` | Shell bootstrap behavior must keep working for SSH/tmux flows. |
| `app/src/terminal/view/open_in_warp.rs`, `inline_banner/open_in_warp.rs`, tests | `open_in_warp`, `Open in Warp` | `open_in_twarp`, `Open in Twarp` | `rename` | 9f owns visible copy; 9i owns module/action names. |
| `app/Cargo.toml` feature flags | `warp_packs`, `open_warp_launch_modal`, `open_warp_new_settings_modes`, `warpify_footer` | `twarp_packs`, `open_twarp_launch_modal`, etc. | `rename` | Feature flag compatibility may need a bridge if remote config ever references old names. |
| `.warp/**`, `.warpindexingignore`, `WARP.md`, README references to `WARP.md` | repo workflow/config docs | `.twarp/**`, `.twarpindexingignore`, `TWARP.md` | `rename` | Old names can remain only as documented compatibility aliases. |
| `app/resources/tab_configs/**`, SSH install scripts | `~/.warp/...`, tmux socket `-Lwarp` | `~/.twarp/...`, twarp tmux socket if safe | `rename` | Existing remote installs should not be destructively removed. |
| `app/assets/bundled/bootstrap/**`, terminal bootstrap Rust/tests | `WARP_SESSION_ID`, `WARP_BOOTSTRAPPED`, `WARP_HONOR_PS1`, `WARP_*`, OSC 9278 hook names/comments | keep by default; optionally add `TWARP_*` aliases with tests | `keep` | These are protocol names. Rename only with compatibility coverage for bash/zsh/fish/pwsh/SSH/subshell. |
| `app/src/auth/auth_state.rs` | `WARP_USER_SECRET` | `TWARP_USER_SECRET` plus optional old alias | `rename` | Not a shell protocol variable; preserve old alias only for dev compatibility. |
| `crates/persistence/migrations/*warp_drive*`, `*warp_pack*` | migration directory names/schema fragments | keep directory/migration identity; rename surrounding user/Rust labels if safe | `keep` | Applied migration identity should not change. |

Out of scope for 9i: crate/package names already owned by 9b and server endpoint behavior owned by 9h.

## 9j - Cleanup sweep and intentional keep list

| File/pattern | Current | Target | Class | Notes |
| --- | --- | --- | --- | --- |
| `LICENSE-AGPL`, `LICENSE-MIT`, third-party license files | Warp/upstream license text and copyright holders | unchanged | `keep` | Required legal attribution. |
| Copyright notices on Warp-authored files | "Warp" as copyright holder | unchanged | `keep` | Do not rewrite authorship. |
| `README.md`, `CONTRIBUTING.md`, `SECURITY.md`, `AGENTS.md` provenance sections | "fork of Warp", upstream repo/commit/source context | keep explicit attribution; replace current product instructions | `keep` | Nominative use is expected; avoid implying endorsement. |
| `warpdotdev/warp@d0f045c0`, `warpdotdev/warp-proto-apis`, upstream issue/docs URLs | upstream source/dependency references | unchanged unless active user-facing docs need replacement | `keep` | External provenance and dependency URLs are not product branding. |
| `.agents/skills/**`, old fleet/skill docs | historical Warp/twarp workflow references | mostly keep | `keep` | Internal historical docs should not block product rebrand unless they are current user instructions. |
| `roadmap/**`, `specs/**` | historical feature specs/status references | keep unless current visible docs | `keep` | Preserve feature history. |
| Any remaining match after 9b-9i | unclassified | update to Twarp or add explicit keep reason | `replace` | 9j owns final search closure and must not leave unexplained hits. |

Out of scope for 9j until the end: broad refactors that belong to 9b-9i. 9j should verify, not redo, prior phases.

## Dangerous compatibility names

These matches are intentionally not blind search-and-replace targets:

| Pattern | Paths | Initial class | Reason |
| --- | --- | --- | --- |
| `WARP_*` shell/session variables | `app/assets/bundled/bootstrap/**`, `app/assets/bundled/ssh/**`, terminal bootstrap handlers/tests | `keep` | Public shell integration protocol. 9i may add `TWARP_*` aliases, but old names need compatibility tests. |
| OSC 9278 hook names and comments | shell bootstrap scripts, terminal event handling, `warp://cli-agent` path | `keep`/`replace` | Hook protocol should stay stable unless bridged; URI sentinel should become `twarp://` with optional `warp://` fallback. |
| Telemetry event/schema names | `app/src/server/telemetry/**`, `app/src/code_review/telemetry_event.rs`, notebooks telemetry | `keep` | Serialized schemas may be historical contracts. Since telemetry is disabled for OSS/local, do not churn names cosmetically. |
| DB migration names | `crates/persistence/migrations/*warp_drive*`, `*warp_pack*` | `keep` | Renaming applied migrations can invalidate existing dev/user databases. |
| External dependency names | `warp_multi_agent_api`, `warpdotdev/warp-proto-apis`, third-party upstream URLs | `keep` | They name upstream/external projects, not this product. |
| Old `warp://` links | URI parser/tests/comments/help | `replace` by default; `keep` only as fallback | Canonical scheme is `twarp://`. Any old scheme support must be secondary and tested. |

## What stays Warp

- `LICENSE-AGPL`, `LICENSE-MIT`, and third-party license files.
- Copyright notices where Warp or upstream contributors are the copyright holders.
- README/provenance text that says Twarp is a fork of Warp and references the upstream source commit.
- Upstream repository, issue, docs, and dependency URLs when retained for provenance or support context.
- Historical roadmap/spec text and comments that clearly refer to upstream behavior or migration context.
- Protocol compatibility names explicitly listed above, until a later phase implements and tests Twarp aliases.

## Verification for this audit

9a validation is documentation-only:

```sh
test -f roadmap/09-rebrand/AUDIT.md
git grep -n -I -E '\bWarp\b|\bwarp\b|\bWARP\b|warp_|warp-|warpui|dev\.warp|warp://|\.warpindexingignore|WARP\.md' -- . ':(exclude)roadmap/09-rebrand/AUDIT.md' | wc -l
git grep -l -I -E '\bWarp\b|\bwarp\b|\bWARP\b|warp_|warp-|warpui|dev\.warp|warp://|\.warpindexingignore|WARP\.md' -- . ':(exclude)roadmap/09-rebrand/AUDIT.md' | wc -l
git ls-files | grep -E '(^|/)(.*[Ww]arp.*|\.warp.*|WARP\.md)$' | wc -l
```

Expected baseline at `c3c517b6`:

- Matching text lines: `10,137`
- Matching text files: `1,808`
- Matching filename/path hits: `765`
