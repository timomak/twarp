# 09 - Rebrand to twarp (PRODUCT)

Companion to [TECH.md](TECH.md). Behavior is written as numbered, testable invariants. The feature is intentionally sub-phased because it touches crate names, binary names, bundle metadata, assets, installers, server endpoints, user-visible strings, persisted paths, and repository hygiene.

## Summary

Rebrand this fork from Warp to Twarp everywhere the user, operating system, build output, or developer tooling presents this product as its own app. The finished app installs, launches, stores data, handles links, exposes binaries, and renders UI as Twarp/twarp while preserving legally required attribution to upstream Warp and source-level history where changing it would be misleading or unsafe.

## Goals / Non-goals

**Goals**

- The installed app, CLI binary, URI scheme, bundle identifiers, icon assets, package names, UI copy, help text, settings, about page, and local support files consistently use Twarp/twarp.
- The workspace builds under twarp crate and binary names without a compatibility dependency on old `warp_*` crate identifiers.
- Existing product behavior survives the rename: terminal sessions, panes, tabs, Claude pane, local settings, persisted databases, login-disabled OSS behavior, feature flags, and release scripts keep working under the new names.
- Upstream Warp attribution remains explicit where legally or historically required.

**Non-goals**

- No new user-facing features beyond the rename.
- No revival of removed AI/account/billing surfaces from feature 02.
- No upstream interaction, upstream PR, upstream issue, or push to `warpdotdev/warp`.
- No attempt to erase copyright notices, AGPL/MIT license text, dependency provenance, commit history, or comments that intentionally refer to upstream Warp.

## Behavior

### Identity and attribution

1. The product name shown to users is **Twarp**. Lowercase technical names use **twarp**. No user-facing surface presents the app as "Warp" except attribution text that explicitly says this is a fork of Warp or references upstream Warp as the source project.

2. Legal and historical attribution remains intact:
   - `LICENSE-AGPL` and `LICENSE-MIT` keep their original terms.
   - Copyright notices on Warp-authored source files keep "Warp" where that is the copyright holder.
   - README/provenance text may say "fork of Warp" and may retain upstream repository/commit references.
   - Comments may keep "Warp" when they are clearly about upstream behavior, historical migration context, third-party issue links, or trademark/legal attribution.

3. The completed rebrand must not imply endorsement by Warp, warpdotdev, or upstream maintainers. Any remaining "Warp" reference must be classed as a deliberate `keep` in the audit, not an overlooked brand occurrence.

4. Existing twarp notes already present in code comments remain "twarp" and continue to identify fork-specific changes. They are not converted back to "Warp" or removed as part of cleanup.

### 9a - Audit doc

5. `roadmap/09-rebrand/AUDIT.md` enumerates every tracked file or identifier that contains `Warp`, `warp`, `WARP`, or a Warp-branded asset/path. Each item is classified as `rename`, `replace`, `keep`, or `regenerate`, with a short reason.

6. The audit is actionable for later workers: each sub-phase 9b-9j has a section listing the files it owns, the exact old/new naming pattern, and any items explicitly out of scope for that sub-phase.

7. The audit distinguishes product branding from protocol/internal compatibility names. In particular, shell environment variables, OSC hook names, telemetry event schema names, database migration identifiers, and third-party URLs are not blindly renamed unless the audit explains why doing so is safe.

### 9b - Workspace and crate renames

8. Cargo package, library, crate, and dependency names that are product-branded change from `warp_*` to `twarp_*`, from `warpui*` to `twarpui*`, and from package/lib `warp` to `twarp`.

9. The default developer workflow builds with the new crate graph. A developer can run the twarp dev binary without importing old `warp_*` crate names from local code.

10. Rust module paths, `use` statements, feature references, test package selectors, and workspace dependency keys are internally consistent after the crate rename. No duplicate compatibility crates exist solely to preserve old crate names unless an audit item explicitly requires a temporary shim.

### 9c - CLI binary and URI scheme

11. The default OSS binary is named `twarp-oss`; the local app binary is named `twarp`; any user-facing command examples use `twarp` or `twarp-oss`.

12. Native desktop intents use the `twarp://` scheme. Old `warp://` examples, tests, parser comments, and user-facing help text are updated unless the audit classifies a compatibility path as intentionally retained.

13. If backward compatibility for old `warp://` links is retained, it is explicit and secondary: `twarp://` is the canonical scheme, and any `warp://` handling exists only as a migration fallback with no new UI promoting it.

14. Shell/editor integration, command palette actions, deep links, CLI-agent sentinels, and settings links continue to route to the same behavior under the new canonical scheme.

### 9d - Bundle IDs and native plists

15. Bundle identifiers, app IDs, D-Bus service names, application support paths, and native plist metadata use the `dev.twarp.*` namespace for all release channels unless a later owner-supplied domain supersedes it before implementation.

16. Channel display names are:
   - Stable: `Twarp`
   - Preview: `TwarpPreview`
   - Dev: `TwarpDev`
   - Local: `TwarpLocal`
   - OSS: `TwarpOss`

17. macOS embedded plists, cargo bundle metadata, app service names, Windows single-instance identifiers, and path derivation agree on the same channel identities. Installing multiple channels keeps their data isolated as before, but under twarp names.

18. Existing user data in old Warp-named directories is not silently deleted. If a sub-phase changes a persisted path, the implementation must either migrate/copy the old local data or intentionally start with a new Twarp profile and document that in the audit and smoke test.

### 9e - Brand assets

19. App icons, logo SVGs, splash/onboarding/about logos, package icons, DockTilePlugin resources, and channel-specific icon sets show Twarp branding, not Warp wordmarks or Warp badge assets.

20. The new assets may be simple, but they must be first-party twarp assets. They must not reuse Warp's wordmark or "Built with Warp" badges as product branding.

21. Channel-specific icons remain visually distinguishable where they were distinguishable before (stable/local/dev/preview/oss), and every asset reference points to a real file.

22. The DockTilePlugin keeps supporting user-selectable app icons on macOS, but its bundle, executable, class/file names, resource lookup, and app references are renamed consistently to Twarp.

### 9f - User-facing strings

23. Menus, settings, about page, onboarding, empty states, errors, command palette labels, tooltips, help text, and accessible labels refer to the product as Twarp.

24. "Warp Drive", "Warpify", "Open in Warp", and similar product-fragment labels are renamed to their twarp equivalents if the feature remains visible after feature 02. Removed or hidden surfaces are not revived just to rename them.

25. User-facing documentation links that still point to upstream Warp are either replaced with twarp-owned documentation, removed, or labeled as upstream Warp documentation when retained for provenance/support context.

26. The about/provenance surface makes the fork status clear: Twarp is a community fork of Warp, with upstream license/provenance preserved. It must not look like an official Warp build.

### 9g - Build scripts and installers

27. macOS DMGs/app bundles, Linux AppImage/deb/rpm/Arch packages, Windows installers, desktop entries, package metadata, generated filenames, install/update commands, and CI artifact names use twarp naming.

28. Installer upgrades, package manager commands, desktop launchers, app icons, and uninstall entries do not mix Twarp display names with Warp package IDs.

29. Release workflows and local bundle scripts still support all existing channels that remain in the fork. They upload or emit twarp-named artifacts and do not publish to upstream Warp release buckets by default.

### 9h - Servers and telemetry

30. Twarp does not talk to upstream Warp production services by accident. Remaining upstream endpoints are either removed, disabled, redirected to twarp-owned endpoints, or retained only where the audit explains that the code path is unreachable in the shipped OSS/local build.

31. Auto-update is disabled unless a twarp-owned release endpoint is configured. Users should not see or trigger an update flow that downloads Warp builds.

32. Telemetry and crash reporting remain disabled for OSS/local builds unless a twarp-owned destination is deliberately configured in a future feature. Queued telemetry filenames and privacy copy should not claim Warp collection.

33. Login, auth, cloud sync, Warp Drive sharing, and billing endpoints removed or darkened by feature 02 remain dark. The rebrand does not make those upstream services reachable again.

### 9i - Internal feature names and local files

34. Internal feature names that are product-branded are renamed for consistency where doing so does not break persisted compatibility: `warp_drive` -> `twarp_drive`, `warpify` -> `twarpify`, `warp_pack` -> `twarp_pack`, `open_in_warp` -> `open_in_twarp`.

35. User-visible repo/runtime files are renamed: `.warp/` -> `.twarp/`, `.warpindexingignore` -> `.twarpindexingignore`, and `WARP.md` -> `TWARP.md`.

36. Shell protocol environment variables and OSC hook names may remain `WARP_*` only if the audit classifies them as compatibility protocol names. If they are renamed, existing shell integration and SSH bootstrap behavior must continue to work for new twarp sessions.

37. Database migration directory names or persisted schema identifiers that include Warp fragments are kept when renaming them would risk invalidating applied migrations. User-visible labels and Rust type names around those migrations still move to Twarp where safe.

### 9j - Cleanup sweep

38. The final cleanup sweep runs a whole-repo search for case-sensitive and case-insensitive Warp references and classifies every remaining match against the audit.

39. No unclassified product-branding match remains in tracked text files, generated metadata, bundle manifests, or asset filenames. Binary assets are verified by filename/reference and visual inspection rather than raw text search alone.

40. The final build and smoke test use the renamed binary (`twarp-oss` for OSS/dev validation) and confirm that the old `warp-oss` command is no longer the canonical success path.

## Smoke test

Run against a freshly built twarp binary. Use the default OSS build unless the sub-phase specifically needs a release bundle: `cargo build --bin twarp-oss` after 9c, or the pre-9c equivalent while implementing earlier sub-phases.

### 9a - Audit doc

1. Open `roadmap/09-rebrand/AUDIT.md`. Confirm it has sections for 9b through 9j and every section lists file patterns plus `rename` / `replace` / `keep` / `regenerate` decisions.
2. Run the audit search command recorded in `AUDIT.md`. Confirm its remaining count matches the audit's summary and every sample match has a classification.

### 9b - Workspace and crate renames

3. Run `cargo metadata --format-version=1`. Confirm workspace packages use `twarp`, `twarp_*`, and `twarpui*` names instead of product-branded `warp_*` / `warpui*` names.
4. Run `cargo build --bin twarp-oss` once 9c has landed, or the current default binary during 9b. Confirm no local source import still requires an old `warp_*` crate name.

### 9c - CLI binary and URI scheme

5. Run `cargo build --bin twarp-oss` and confirm the built executable is `twarp-oss`.
6. Launch twarp and open a canonical `twarp://` action/settings deep link. Confirm it routes to the same destination that the old Warp link used to route to.
7. Search user-facing help and examples for `warp://`. Confirm none remain unless documented as compatibility fallback.

### 9d - Bundle IDs and native plists

8. Inspect the generated or embedded macOS plist for each channel. Confirm display names and identifiers use `Twarp*` and `dev.twarp.*`.
9. On Linux, inspect the generated D-Bus/desktop identity. Confirm service/path/app names use twarp naming and do not reference `dev.warp`.
10. Launch a renamed build with an existing old local data directory present. Confirm the app either migrates data or starts a documented new Twarp profile without deleting the old data.

### 9e - Brand assets

11. Launch the app and open the about/onboarding/splash surfaces that show product logos. Confirm they show Twarp assets, not Warp wordmarks.
12. Build or inspect channel icon outputs. Confirm stable/local/dev/preview/oss icons resolve to existing twarp-branded files.
13. On macOS, change the app icon if that setting is available. Quit and relaunch. Confirm the renamed DockTilePlugin preserves the selected icon.

### 9f - User-facing strings

14. Open menus, settings, about, command palette, empty states, and common error/help surfaces. Confirm product copy says Twarp/twarp.
15. Confirm fork attribution copy says Twarp is a fork of Warp and does not imply official Warp endorsement.
16. Confirm no removed AI/account/billing UI reappears while renaming old copy.

### 9g - Build scripts and installers

17. Run bundle check commands available on the current platform, at minimum the headless bundle check from CI for OSS if supported. Confirm generated artifact names use twarp.
18. Inspect Linux desktop/package metadata or Windows installer metadata for the built channel. Confirm package ID, display name, icon name, and executable name are consistently twarp.
19. Confirm release workflow artifacts and upload paths no longer publish twarp builds under Warp filenames or upstream Warp release bucket names by default.

### 9h - Servers and telemetry

20. Launch OSS/local twarp with network logging or config inspection. Confirm telemetry and crash reporting are disabled and no upstream Warp telemetry endpoint is configured.
21. Trigger the update-check UI/path if available. Confirm it is disabled or points to a twarp-owned endpoint, never an upstream Warp build.
22. Exercise login/cloud-related surfaces that remain visible after feature 02. Confirm they are disabled, local-only, or twarp-owned; they must not silently call upstream Warp production services.

### 9i - Internal feature names and local files

23. Confirm `.twarp/`, `.twarpindexingignore`, and `TWARP.md` are the documented repo/runtime names. Old `.warp*`/`WARP.md` names remain only as explicitly documented compatibility aliases if implemented.
24. Run tests or manual flows around shell bootstrap, SSH, local settings, and codebase indexing. Confirm protocol variables kept as `WARP_*` still work if classified as compatibility, or their twarp replacements work if renamed.
25. Confirm database migrations do not rerun destructively or lose existing local objects solely because a migration/path was renamed.

### 9j - Cleanup sweep

26. Run the final whole-repo Warp search recorded in `AUDIT.md`. Confirm every remaining match is legal attribution, upstream provenance, historical copyright, third-party URL, compatibility protocol, or another explicit `keep`.
27. Run `cargo fmt -- --check`, `cargo build --bin twarp-oss`, and `cargo clippy --workspace -- -D warnings`.
28. Launch twarp. Confirm the running app, menu bar, dock/taskbar entry, about page, URI scheme, generated log/data path, and executable name all present as Twarp/twarp.
