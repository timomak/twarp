//! twarp 20c: the twarp-managed shared-skills store.
//!
//! Skills live in `~/.twarp/skills/<name>/SKILL.md` (content on disk is the
//! source of truth) and are MATERIALIZED to both providers:
//! - Claude: `~/.claude/skills/<name>` symlink into the store (skipped and
//!   surfaced as a conflict when a real directory already occupies the name).
//! - Codex: the SKILL.md body (frontmatter stripped) rendered to
//!   `~/.codex/prompts/<name>.md` with a leading ownership marker so twarp
//!   only ever overwrites / deletes files it wrote itself.
//!
//! Per-provider enable toggles are kept in the `shared_skills` SQLite table;
//! disabled = the symlink / prompt file is removed. The materializer is
//! idempotent, repairs broken symlinks, and garbage-collects managed artifacts
//! for skills no longer in the store. All filesystem work runs on the
//! background executor — never on the main thread.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use twarpui::{Entity, ModelContext, SingletonEntity};

use crate::persistence::{ModelEvent, PersistedSharedSkill};
use crate::GlobalResourceHandlesProvider;

/// Leading marker identifying Codex prompt files owned by twarp.
pub const CODEX_MANAGED_MARKER: &str = "<!-- managed by twarp -->";

/// Filesystem roots the store operates on; injectable for tests.
#[derive(Clone, Debug)]
pub struct SkillsPaths {
    /// `~/.twarp/skills` — the store itself.
    pub store: PathBuf,
    /// `~/.claude/skills` — Claude's user-level skills directory.
    pub claude_skills: PathBuf,
    /// `~/.codex/prompts` — Codex's user-level custom-prompt directory.
    pub codex_prompts: PathBuf,
}

impl SkillsPaths {
    fn from_home() -> Option<Self> {
        let home = dirs::home_dir()?;
        Some(Self {
            store: home.join(".twarp/skills"),
            claude_skills: home.join(".claude/skills"),
            codex_prompts: home.join(".codex/prompts"),
        })
    }
}

/// One skill in the store, as shown on the Skills page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillEntry {
    /// Directory name, doubling as the `/name` invocation.
    pub name: String,
    /// Frontmatter `description`, or empty when absent.
    pub description: String,
    pub enabled_claude: bool,
    pub enabled_codex: bool,
    /// A real (non-symlink) `~/.claude/skills/<name>` blocks the symlink.
    pub claude_conflict: bool,
    /// An unmanaged `~/.codex/prompts/<name>.md` blocks the rendered prompt.
    pub codex_conflict: bool,
}

/// A real (non-symlink) skill directory in `~/.claude/skills` that can be
/// adopted: moved into the store with a symlink left in its place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdoptableSkill {
    pub name: String,
    pub path: PathBuf,
}

/// What one background scan + materialize pass found.
#[derive(Debug, Default)]
struct ScanResult {
    skills: Vec<ScannedSkill>,
    adoptable: Vec<AdoptableSkill>,
}

#[derive(Debug)]
struct ScannedSkill {
    name: String,
    description: String,
    claude_conflict: bool,
    codex_conflict: bool,
}

/// Parse loose SKILL.md frontmatter. Returns `(name, description, body)`;
/// tolerates missing frontmatter (whole content becomes the body) and
/// unquoted / loosely formatted YAML values.
fn parse_frontmatter(content: &str) -> (Option<String>, Option<String>, String) {
    let rest = match content.strip_prefix("---") {
        Some(rest) if rest.starts_with('\n') || rest.starts_with("\r\n") => rest,
        _ => return (None, None, content.to_owned()),
    };
    let Some(end) = rest.find("\n---") else {
        return (None, None, content.to_owned());
    };
    let (frontmatter, tail) = rest.split_at(end);
    // `tail` is "\n---<rest of fence line>\n<body>"; drop the fence line.
    let body = tail
        .trim_start_matches('\n')
        .split_once('\n')
        .map(|(_fence_line, body)| body.to_owned())
        .unwrap_or_default();

    let mut name = None;
    let mut description = None;
    for line in frontmatter.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim().trim_matches('"').trim_matches('\'').to_owned();
        match key.trim() {
            "name" => name = Some(value),
            "description" => description = Some(value),
            _ => {}
        }
    }
    (name, description, body)
}

/// Render a SKILL.md into the Codex custom-prompt file: the ownership marker,
/// then the body with frontmatter stripped.
fn render_codex_prompt(skill_md: &str) -> String {
    let (_, _, body) = parse_frontmatter(skill_md);
    format!("{CODEX_MANAGED_MARKER}\n{}", body.trim_start_matches('\n'))
}

/// Whether an existing Codex prompt file is one twarp wrote (and so may be
/// overwritten / deleted).
fn codex_prompt_is_managed(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .map(|content| content.starts_with(CODEX_MANAGED_MARKER))
        .unwrap_or(false)
}

/// Validate a new-skill name: kebab-case (`[a-z0-9]` runs joined by single
/// hyphens).
pub fn is_valid_skill_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// One scan + materialize pass. Idempotent; runs on a background thread.
fn scan_and_materialize(
    paths: &SkillsPaths,
    toggles: &BTreeMap<String, (bool, bool)>,
) -> ScanResult {
    let mut result = ScanResult::default();

    if let Err(err) = std::fs::create_dir_all(&paths.store) {
        log::error!("skills store: failed to create {:?}: {err}", paths.store);
        return result;
    }

    // Scan the store.
    let mut store_names = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&paths.store) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let skill_md = path.join("SKILL.md");
            if !skill_md.is_file() {
                continue;
            }
            let content = std::fs::read_to_string(&skill_md).unwrap_or_default();
            let (_, description, _) = parse_frontmatter(&content);
            store_names.push(name.clone());
            let (enabled_claude, enabled_codex) =
                toggles.get(&name).copied().unwrap_or((true, true));

            let claude_conflict =
                materialize_claude(paths, &name, enabled_claude).unwrap_or_default();
            let codex_conflict =
                materialize_codex(paths, &name, &content, enabled_codex).unwrap_or_default();

            result.skills.push(ScannedSkill {
                name,
                description: description.unwrap_or_default(),
                claude_conflict,
                codex_conflict,
            });
        }
    }
    result.skills.sort_by(|a, b| a.name.cmp(&b.name));

    // Garbage-collect managed artifacts for skills no longer in the store,
    // and collect adoptable real skill dirs while walking ~/.claude/skills.
    if let Ok(entries) = std::fs::read_dir(&paths.claude_skills) {
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if metadata.is_symlink() {
                let points_into_store = std::fs::read_link(&path)
                    .map(|target| target.starts_with(&paths.store))
                    .unwrap_or_default();
                if points_into_store && !store_names.contains(&name) {
                    let _ = std::fs::remove_file(&path);
                }
            } else if metadata.is_dir()
                && path.join("SKILL.md").is_file()
                && !store_names.contains(&name)
            {
                result.adoptable.push(AdoptableSkill { name, path });
            }
        }
    }
    result.adoptable.sort_by(|a, b| a.name.cmp(&b.name));

    if let Ok(entries) = std::fs::read_dir(&paths.codex_prompts) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()).map(str::to_owned) else {
                continue;
            };
            if !store_names.contains(&stem) && codex_prompt_is_managed(&path) {
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    result
}

/// Ensure / remove the Claude symlink for one skill. Returns `Ok(true)` when a
/// real directory occupies the name (conflict — left untouched).
fn materialize_claude(paths: &SkillsPaths, name: &str, enabled: bool) -> std::io::Result<bool> {
    let link = paths.claude_skills.join(name);
    let target = paths.store.join(name);

    match std::fs::symlink_metadata(&link) {
        Ok(metadata) if metadata.is_symlink() => {
            let current = std::fs::read_link(&link).unwrap_or_default();
            if !enabled || current != target {
                // Disabled, broken, or pointing elsewhere within our
                // management: remove (recreated below when enabled). Only
                // symlinks are ever removed here.
                if enabled || current.starts_with(&paths.store) || !link.exists() {
                    std::fs::remove_file(&link)?;
                } else {
                    // A user-made symlink to somewhere else; leave it alone.
                    return Ok(true);
                }
            } else {
                return Ok(false);
            }
        }
        Ok(_) => return Ok(true), // Real file/dir occupies the name.
        Err(_) => {}
    }

    if enabled {
        std::fs::create_dir_all(&paths.claude_skills)?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &link)?;
    }
    Ok(false)
}

/// Write / remove the rendered Codex prompt for one skill. Returns `Ok(true)`
/// when an unmanaged file occupies the name (conflict — left untouched).
fn materialize_codex(
    paths: &SkillsPaths,
    name: &str,
    skill_md: &str,
    enabled: bool,
) -> std::io::Result<bool> {
    let prompt = paths.codex_prompts.join(format!("{name}.md"));
    let exists = prompt.is_file();
    if exists && !codex_prompt_is_managed(&prompt) {
        return Ok(true);
    }
    if !enabled {
        if exists {
            std::fs::remove_file(&prompt)?;
        }
        return Ok(false);
    }
    let rendered = render_codex_prompt(skill_md);
    if !exists || std::fs::read_to_string(&prompt).ok().as_deref() != Some(rendered.as_str()) {
        std::fs::create_dir_all(&paths.codex_prompts)?;
        std::fs::write(&prompt, rendered)?;
    }
    Ok(false)
}

/// Adopt a real skill dir from `~/.claude/skills`: move it into the store and
/// leave a symlink in its place.
fn adopt_skill(paths: &SkillsPaths, name: &str) -> std::io::Result<()> {
    let source = paths.claude_skills.join(name);
    let dest = paths.store.join(name);
    if std::fs::symlink_metadata(&source)?.is_symlink() || dest.exists() {
        return Err(std::io::Error::other("skill is not adoptable"));
    }
    std::fs::create_dir_all(&paths.store)?;
    std::fs::rename(&source, &dest)?;
    #[cfg(unix)]
    std::os::unix::fs::symlink(&dest, &source)?;
    Ok(())
}

/// Create a new store skill from the inline form: dir + templated SKILL.md.
fn create_skill(paths: &SkillsPaths, name: &str, description: &str) -> std::io::Result<()> {
    let dir = paths.store.join(name);
    if dir.exists() {
        return Err(std::io::Error::other("skill already exists"));
    }
    std::fs::create_dir_all(&dir)?;
    let content = format!(
        "---\nname: {name}\ndescription: {description}\n---\n\n# {name}\n\nDescribe what this skill should do here.\n"
    );
    std::fs::write(dir.join("SKILL.md"), content)
}

/// Delete a skill: store dir plus both managed artifacts.
fn delete_skill(paths: &SkillsPaths, name: &str) {
    let dir = paths.store.join(name);
    if let Err(err) = std::fs::remove_dir_all(&dir) {
        log::error!("skills store: failed to delete {dir:?}: {err}");
    }
    let link = paths.claude_skills.join(name);
    if std::fs::symlink_metadata(&link).is_ok_and(|m| m.is_symlink()) {
        let _ = std::fs::remove_file(&link);
    }
    let prompt = paths.codex_prompts.join(format!("{name}.md"));
    if prompt.is_file() && codex_prompt_is_managed(&prompt) {
        let _ = std::fs::remove_file(&prompt);
    }
}

/// Singleton owning the in-memory view of the store. All filesystem work runs
/// on the background executor; scan results stream back over a channel and are
/// applied on the foreground.
pub struct SkillsStoreModel {
    paths: Option<SkillsPaths>,
    skills: Vec<SkillEntry>,
    adoptable: Vec<AdoptableSkill>,
    /// name -> (enabled_claude, enabled_codex); persisted to `shared_skills`.
    toggles: BTreeMap<String, (bool, bool)>,
    scan_tx: async_channel::Sender<ScanResult>,
}

impl SkillsStoreModel {
    pub fn new(persisted: Vec<PersistedSharedSkill>, ctx: &mut ModelContext<Self>) -> Self {
        let (scan_tx, scan_rx) = async_channel::unbounded::<ScanResult>();
        let _ = ctx.spawn_stream_local(
            scan_rx,
            |model: &mut Self, result, ctx| model.apply_scan(result, ctx),
            |_, _| {},
        );
        let mut model = Self {
            paths: SkillsPaths::from_home(),
            skills: Vec::new(),
            adoptable: Vec::new(),
            toggles: persisted
                .into_iter()
                .map(|row| (row.name, (row.enabled_claude, row.enabled_codex)))
                .collect(),
            scan_tx,
        };
        model.request_refresh(ctx);
        model
    }

    pub fn skills(&self) -> &[SkillEntry] {
        &self.skills
    }

    pub fn adoptable(&self) -> &[AdoptableSkill] {
        &self.adoptable
    }

    /// Absolute path of a skill's directory in the store (for Reveal).
    pub fn skill_dir(&self, name: &str) -> Option<PathBuf> {
        self.paths.as_ref().map(|paths| paths.store.join(name))
    }

    pub fn name_taken(&self, name: &str) -> bool {
        self.skills.iter().any(|s| s.name == name)
    }

    /// Kick off a background scan + materialize pass.
    pub fn request_refresh(&self, ctx: &mut ModelContext<Self>) {
        self.spawn_fs_task(ctx, |_| {});
    }

    /// Flip one provider-enable bit, persist, and re-materialize.
    pub fn toggle_enabled(&mut self, name: &str, claude: bool, ctx: &mut ModelContext<Self>) {
        let entry = self.toggles.entry(name.to_owned()).or_insert((true, true));
        if claude {
            entry.0 = !entry.0;
        } else {
            entry.1 = !entry.1;
        }
        if let Some(skill) = self.skills.iter_mut().find(|s| s.name == name) {
            skill.enabled_claude = self.toggles[name].0;
            skill.enabled_codex = self.toggles[name].1;
        }
        self.persist_toggles(ctx);
        self.request_refresh(ctx);
    }

    /// Create a new skill from the inline form, then rescan.
    pub fn create(&self, name: String, description: String, ctx: &mut ModelContext<Self>) {
        self.spawn_fs_task(ctx, move |paths| {
            if let Err(err) = create_skill(paths, &name, &description) {
                log::error!("skills store: failed to create skill {name}: {err}");
            }
        });
    }

    /// Delete a skill (store dir + managed artifacts), then rescan.
    pub fn delete(&mut self, name: &str, ctx: &mut ModelContext<Self>) {
        if self.toggles.remove(name).is_some() {
            self.persist_toggles(ctx);
        }
        let name = name.to_owned();
        self.spawn_fs_task(ctx, move |paths| delete_skill(paths, &name));
    }

    /// Adopt a real `~/.claude/skills` dir into the store, then rescan.
    pub fn adopt(&self, name: &str, ctx: &mut ModelContext<Self>) {
        let name = name.to_owned();
        self.spawn_fs_task(ctx, move |paths| {
            if let Err(err) = adopt_skill(paths, &name) {
                log::error!("skills store: failed to adopt skill {name}: {err}");
            }
        });
    }

    /// Run `work` then a scan + materialize pass on the background executor;
    /// the resulting snapshot streams back to `apply_scan`.
    fn spawn_fs_task(
        &self,
        ctx: &mut ModelContext<Self>,
        work: impl FnOnce(&SkillsPaths) + Send + 'static,
    ) {
        let Some(paths) = self.paths.clone() else {
            return;
        };
        let toggles = self.toggles.clone();
        let tx = self.scan_tx.clone();
        ctx.background_executor()
            .spawn(async move {
                work(&paths);
                let result = scan_and_materialize(&paths, &toggles);
                let _ = tx.send(result).await;
            })
            .detach();
    }

    fn apply_scan(&mut self, result: ScanResult, ctx: &mut ModelContext<Self>) {
        self.skills = result
            .skills
            .into_iter()
            .map(|skill| {
                let (enabled_claude, enabled_codex) = self
                    .toggles
                    .get(&skill.name)
                    .copied()
                    .unwrap_or((true, true));
                SkillEntry {
                    name: skill.name,
                    description: skill.description,
                    enabled_claude,
                    enabled_codex,
                    claude_conflict: skill.claude_conflict,
                    codex_conflict: skill.codex_conflict,
                }
            })
            .collect();
        self.adoptable = result.adoptable;
        ctx.notify();
    }

    fn persist_toggles(&self, ctx: &mut ModelContext<Self>) {
        ctx.notify();
        let handles = GlobalResourceHandlesProvider::as_ref(ctx).get();
        if let Some(sender) = &handles.model_event_sender {
            let event = ModelEvent::ReplaceSharedSkills {
                skills: self
                    .toggles
                    .iter()
                    .map(
                        |(name, (enabled_claude, enabled_codex))| PersistedSharedSkill {
                            name: name.clone(),
                            enabled_claude: *enabled_claude,
                            enabled_codex: *enabled_codex,
                        },
                    )
                    .collect(),
            };
            if let Err(err) = sender.send(event) {
                log::error!("Failed to persist shared skills toggles: {err}");
            }
        }
    }
}

impl Entity for SkillsStoreModel {
    type Event = ();
}

impl SingletonEntity for SkillsStoreModel {}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(root: &Path) -> SkillsPaths {
        SkillsPaths {
            store: root.join("twarp/skills"),
            claude_skills: root.join("claude/skills"),
            codex_prompts: root.join("codex/prompts"),
        }
    }

    fn write_skill(dir: &Path, name: &str, content: &str) {
        let skill_dir = dir.join(name);
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), content).unwrap();
    }

    const SKILL_MD: &str =
        "---\nname: my-skill\ndescription: Does things.\n---\n\n# My skill\n\nBody text.\n";

    #[test]
    fn frontmatter_parses_name_and_description() {
        let (name, description, body) = parse_frontmatter(SKILL_MD);
        assert_eq!(name.as_deref(), Some("my-skill"));
        assert_eq!(description.as_deref(), Some("Does things."));
        assert_eq!(body, "\n# My skill\n\nBody text.\n");
    }

    #[test]
    fn frontmatter_tolerates_missing_and_loose_yaml() {
        let (name, description, body) = parse_frontmatter("just a body\n");
        assert_eq!(name, None);
        assert_eq!(description, None);
        assert_eq!(body, "just a body\n");

        let loose =
            "---\nname: \"quoted-name\"\nnot yaml at all\ndescription: has: colons\n---\nbody";
        let (name, description, _) = parse_frontmatter(loose);
        assert_eq!(name.as_deref(), Some("quoted-name"));
        assert_eq!(description.as_deref(), Some("has: colons"));
    }

    #[test]
    fn codex_render_strips_frontmatter_and_adds_marker() {
        let rendered = render_codex_prompt(SKILL_MD);
        assert!(rendered.starts_with(CODEX_MANAGED_MARKER));
        assert!(rendered.contains("# My skill"));
        assert!(!rendered.contains("description:"));
    }

    #[test]
    fn skill_name_validation() {
        assert!(is_valid_skill_name("my-skill-2"));
        assert!(!is_valid_skill_name(""));
        assert!(!is_valid_skill_name("My-Skill"));
        assert!(!is_valid_skill_name("-lead"));
        assert!(!is_valid_skill_name("trail-"));
        assert!(!is_valid_skill_name("dou--ble"));
        assert!(!is_valid_skill_name("space name"));
    }

    #[test]
    fn materialize_creates_symlink_and_prompt_and_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = paths(tmp.path());
        write_skill(&paths.store, "my-skill", SKILL_MD);

        for _ in 0..2 {
            let result = scan_and_materialize(&paths, &BTreeMap::new());
            assert_eq!(result.skills.len(), 1);
            assert!(!result.skills[0].claude_conflict);
            assert!(!result.skills[0].codex_conflict);
            let link = paths.claude_skills.join("my-skill");
            assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
            assert_eq!(
                std::fs::read_link(&link).unwrap(),
                paths.store.join("my-skill")
            );
            assert!(codex_prompt_is_managed(
                &paths.codex_prompts.join("my-skill.md")
            ));
        }
    }

    #[test]
    fn disabled_skill_artifacts_are_removed_and_conflicts_reported() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = paths(tmp.path());
        write_skill(&paths.store, "my-skill", SKILL_MD);
        scan_and_materialize(&paths, &BTreeMap::new());

        let toggles = BTreeMap::from([("my-skill".to_owned(), (false, false))]);
        scan_and_materialize(&paths, &toggles);
        assert!(std::fs::symlink_metadata(paths.claude_skills.join("my-skill")).is_err());
        assert!(!paths.codex_prompts.join("my-skill.md").exists());

        // A real dir at the Claude location and an unmanaged prompt file are
        // conflicts, never clobbered.
        write_skill(&paths.claude_skills, "my-skill", "user content");
        std::fs::write(paths.codex_prompts.join("my-skill.md"), "user prompt").unwrap();
        let result = scan_and_materialize(&paths, &BTreeMap::new());
        assert!(result.skills[0].claude_conflict);
        assert!(result.skills[0].codex_conflict);
        assert_eq!(
            std::fs::read_to_string(paths.codex_prompts.join("my-skill.md")).unwrap(),
            "user prompt"
        );
    }

    #[test]
    fn orphaned_managed_artifacts_are_garbage_collected() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = paths(tmp.path());
        write_skill(&paths.store, "my-skill", SKILL_MD);
        scan_and_materialize(&paths, &BTreeMap::new());

        std::fs::remove_dir_all(paths.store.join("my-skill")).unwrap();
        scan_and_materialize(&paths, &BTreeMap::new());
        assert!(std::fs::symlink_metadata(paths.claude_skills.join("my-skill")).is_err());
        assert!(!paths.codex_prompts.join("my-skill.md").exists());
    }

    #[test]
    fn adopt_moves_dir_and_leaves_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = paths(tmp.path());
        write_skill(&paths.claude_skills, "adopt-me", SKILL_MD);

        let result = scan_and_materialize(&paths, &BTreeMap::new());
        assert_eq!(result.adoptable.len(), 1);
        assert_eq!(result.adoptable[0].name, "adopt-me");

        adopt_skill(&paths, "adopt-me").unwrap();
        assert!(paths.store.join("adopt-me/SKILL.md").is_file());
        let link = paths.claude_skills.join("adopt-me");
        assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
        assert_eq!(
            std::fs::read_link(&link).unwrap(),
            paths.store.join("adopt-me")
        );

        // Post-adopt scan sees it as a store skill, not adoptable.
        let result = scan_and_materialize(&paths, &BTreeMap::new());
        assert!(result.adoptable.is_empty());
        assert_eq!(result.skills.len(), 1);
        assert!(!result.skills[0].claude_conflict);
    }

    #[test]
    fn create_and_delete_skill_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = paths(tmp.path());
        create_skill(&paths, "new-skill", "A fresh skill.").unwrap();
        let content = std::fs::read_to_string(paths.store.join("new-skill/SKILL.md")).unwrap();
        let (name, description, _) = parse_frontmatter(&content);
        assert_eq!(name.as_deref(), Some("new-skill"));
        assert_eq!(description.as_deref(), Some("A fresh skill."));
        assert!(create_skill(&paths, "new-skill", "dupe").is_err());

        scan_and_materialize(&paths, &BTreeMap::new());
        delete_skill(&paths, "new-skill");
        assert!(!paths.store.join("new-skill").exists());
        assert!(std::fs::symlink_metadata(paths.claude_skills.join("new-skill")).is_err());
        assert!(!paths.codex_prompts.join("new-skill.md").exists());
    }
}
