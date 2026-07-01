//! Composer intelligence for the Claude Code pane (feature 07, sub-phase 7j;
//! PRODUCT §15a–§15b):
//!
//! - **`/` suggestions** — typing a leading `/` filters the session's slash
//!   commands (the `init` message's `slash_commands`, plus a small built-in
//!   set so a fresh pane isn't empty before the first init).
//! - **`@` file mentions** — an `@`-prefixed token fuzzy-filters the cwd's
//!   files (walk includes gitignored files but skips `.git`/dotfiles, capped).
//!   Accepting inserts the relative path; `claude` reads mentioned files itself.
//! - **Image attachments** — `@`-mentions that resolve to image files under
//!   the cwd become real attachments: previewed as chips above the composer
//!   and sent as base64 `image` content blocks (verified accepted by
//!   `claude`'s stream-json input), so the model *sees* the image without a
//!   tool round-trip.
//!
//! The pure parsing/filtering/scanning lives here, unit-tested headlessly;
//! the view owns the state and rendering.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// What the active token asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SuggestionKind {
    SlashCommand,
    FileMention,
}

/// The active suggestion query: what to filter, and the byte range of the
/// token being completed (replaced on accept).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SuggestionQuery {
    pub kind: SuggestionKind,
    pub query: String,
    /// Byte range of the token (including its `/` or `@` sigil) in the
    /// buffer text.
    pub token_range: std::ops::Range<usize>,
}

/// Slash commands that exist on every `claude` regardless of skills/plugins,
/// shown before the first `init` reports the real list (PRODUCT §15a).
pub(super) const DEFAULT_SLASH_COMMANDS: &[&str] = &[
    "clear", "compact", "config", "cost", "doctor", "help", "init", "model", "review", "status",
];

/// Cap on suggestion rows rendered (the list filters as you type).
pub(super) const MAX_SUGGESTIONS: usize = 8;

/// The slash-command catalogue the composer offers under `/` (PRODUCT §15a):
/// command names plus, where we can find one, a one-line description.
///
/// `claude`'s `init` reports the *names* of every command (built-ins + skills +
/// plugins), but no descriptions, and only once a session has started. We scan
/// the on-disk skill directories ourselves so (a) skills show up the moment the
/// pane opens — before the first `init` — and (b) each one carries the
/// description from its `SKILL.md` frontmatter (issues #2/#3).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct SlashCommandIndex {
    /// Command names offered as `/` completions, deduped and sorted.
    pub names: Vec<String>,
    /// `name → description`, for the rows we found a `SKILL.md` description for.
    pub descriptions: HashMap<String, String>,
}

impl SlashCommandIndex {
    /// Merge command names reported by `claude`'s `init` (skills + plugins +
    /// MCP commands we can't see on disk) into the catalogue, keeping it sorted
    /// and deduped. Descriptions already discovered are preserved.
    pub fn merge_names<'a>(&mut self, names: impl IntoIterator<Item = &'a str>) {
        for name in names {
            if !self.names.iter().any(|existing| existing == name) {
                self.names.push(name.to_owned());
            }
        }
        self.names.sort();
        self.names.dedup();
    }
}

/// Build the slash-command catalogue from the built-ins plus the user-level and
/// project-level skill directories (`~/.claude/skills/*/SKILL.md` and
/// `<cwd>/.claude/skills/*/SKILL.md`). Descriptions come from each skill's
/// frontmatter; names with no on-disk skill (built-ins, plugins) simply carry
/// no description.
pub(super) fn build_slash_command_index(cwd: &Path) -> SlashCommandIndex {
    let mut index = SlashCommandIndex::default();
    index.merge_names(DEFAULT_SLASH_COMMANDS.iter().copied());

    let mut skill_roots: Vec<PathBuf> = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        skill_roots.push(Path::new(&home).join(".claude").join("skills"));
    }
    skill_roots.push(cwd.join(".claude").join("skills"));

    for root in skill_roots {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            if !entry.file_type().is_ok_and(|ty| ty.is_dir()) {
                continue;
            }
            let skill_md = entry.path().join("SKILL.md");
            let Ok(contents) = std::fs::read_to_string(&skill_md) else {
                continue;
            };
            // The directory name is the command name `claude` exposes; the
            // frontmatter `name` is usually identical but we trust the dir.
            let name = entry.file_name().to_string_lossy().into_owned();
            index.merge_names(std::iter::once(name.as_str()));
            if let Some(description) = parse_frontmatter_description(&contents) {
                index.descriptions.entry(name).or_insert(description);
            }
        }
    }
    index
}

/// Pull the `description:` value out of a `SKILL.md` YAML frontmatter block
/// (the leading `---`-fenced section). Returns `None` when there's no
/// frontmatter or no description key.
fn parse_frontmatter_description(contents: &str) -> Option<String> {
    let body = contents.strip_prefix("---")?;
    // Frontmatter ends at the next line that is exactly `---`.
    let end = body.find("\n---")?;
    let frontmatter = &body[..end];
    for line in frontmatter.lines() {
        if let Some(value) = line.trim_start().strip_prefix("description:") {
            let value = value.trim().trim_matches('"').trim_matches('\'').trim();
            if !value.is_empty() {
                return Some(value.to_owned());
            }
        }
    }
    None
}

/// Cap on files gathered by the cwd walk — enough for real repos, bounded so
/// a giant tree can't stall the composer.
pub(super) const MAX_CWD_FILES: usize = 5_000;

/// Hard ceiling for one attached image's raw size. The API rejects ~5 MB of
/// base64; 3.75 MB raw encodes right up to that.
pub(super) const MAX_IMAGE_BYTES: u64 = 3_750_000;

/// Derive the active suggestion query from the composer text (PRODUCT §15a).
///
/// The query is always the trailing whitespace-delimited token at the cursor,
/// keyed on its leading sigil:
/// - a `/`-prefixed token opens the slash-command menu, and
/// - an `@`-prefixed token opens the file-mention menu.
///
/// Both fire on the token wherever it sits in the draft, not just at the start
/// — typing `/` mid-message surfaces the catalogue the same as at column zero.
pub(super) fn suggestion_query(text: &str) -> Option<SuggestionQuery> {
    let token_start = text
        .rfind(char::is_whitespace)
        .map(|idx| idx + text[idx..].chars().next().map_or(1, char::len_utf8))
        .unwrap_or(0);
    let token = &text[token_start..];
    if let Some(rest) = token.strip_prefix('/') {
        return Some(SuggestionQuery {
            kind: SuggestionKind::SlashCommand,
            query: rest.to_owned(),
            token_range: token_start..text.len(),
        });
    }
    let rest = token.strip_prefix('@')?;
    Some(SuggestionQuery {
        kind: SuggestionKind::FileMention,
        query: rest.to_owned(),
        token_range: token_start..text.len(),
    })
}

/// Fuzzy-filter `candidates` against `query`, best score first, capped at
/// [`MAX_SUGGESTIONS`]. An empty query keeps the candidates' own order.
pub(super) fn filter_suggestions(query: &str, candidates: &[String]) -> Vec<String> {
    if query.is_empty() {
        return candidates.iter().take(MAX_SUGGESTIONS).cloned().collect();
    }
    let mut scored: Vec<(i64, &String)> = candidates
        .iter()
        .filter_map(|candidate| {
            fuzzy_match::match_indices_case_insensitive(candidate, query)
                .map(|result| (result.score, candidate))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored
        .into_iter()
        .take(MAX_SUGGESTIONS)
        .map(|(_, candidate)| candidate.clone())
        .collect()
}

/// Replace the queried token with the accepted suggestion, returning the new
/// buffer text. The sigil is re-applied and a trailing space commits the
/// token so typing flows on.
pub(super) fn apply_suggestion(text: &str, query: &SuggestionQuery, accepted: &str) -> String {
    let sigil = match query.kind {
        SuggestionKind::SlashCommand => '/',
        SuggestionKind::FileMention => '@',
    };
    let mut new_text = String::with_capacity(text.len() + accepted.len() + 2);
    new_text.push_str(&text[..query.token_range.start]);
    new_text.push(sigil);
    new_text.push_str(accepted);
    new_text.push(' ');
    new_text.push_str(&text[query.token_range.end..]);
    new_text
}

/// Walk the cwd for mentionable files: relative paths, files only, capped at
/// [`MAX_CWD_FILES`] (PRODUCT §15a).
///
/// Gitignored files ARE included so they can be `@`-mentioned — the `.gitignore`
/// / `.ignore` / global-ignore filters are all disabled. Hidden dotfiles stay
/// excluded via `.hidden(true)`, which also keeps the `.git` internal directory
/// out.
pub(super) fn list_cwd_files(cwd: &Path) -> Vec<String> {
    let mut files = Vec::new();
    for entry in ignore::WalkBuilder::new(cwd)
        .hidden(true)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .ignore(false)
        .parents(false)
        .follow_links(false)
        .build()
        .flatten()
    {
        if !entry.file_type().is_some_and(|ty| ty.is_file()) {
            continue;
        }
        if let Ok(relative) = entry.path().strip_prefix(cwd) {
            files.push(relative.display().to_string());
            if files.len() >= MAX_CWD_FILES {
                break;
            }
        }
    }
    files.sort();
    files
}

/// The IANA media type for an image path the API accepts, or `None` for
/// non-image (or unsupported-image) files.
pub(super) fn image_media_type(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

/// Normalize a clipboard image MIME type to the API's accepted set (7l §49),
/// mapping the `image/jpg` alias to `image/jpeg`. `None` for types the API
/// won't take as an inline image block — those degrade like any other
/// unsupported attachment (§51).
pub(super) fn normalize_image_media_type(media_type: &str) -> Option<&'static str> {
    match media_type.to_ascii_lowercase().as_str() {
        "image/png" => Some("image/png"),
        "image/jpeg" | "image/jpg" => Some("image/jpeg"),
        "image/gif" => Some("image/gif"),
        "image/webp" => Some("image/webp"),
        _ => None,
    }
}

/// Format an `@`-mention for `path`, relative to `cwd` when the file lives under
/// it (keeps the draft readable instead of pasting an absolute path), absolute
/// otherwise (7l §50–§51). `claude` resolves both.
pub(super) fn mention_for(path: &Path, cwd: &Path) -> String {
    let shown = path.strip_prefix(cwd).unwrap_or(path);
    format!("@{}", shown.display())
}

/// Append `mention` to the composer `text`, inserting a separating space when
/// the draft doesn't already end in whitespace (7l §50–§51). Returns the new
/// buffer text.
pub(super) fn append_mention(text: &str, mention: &str) -> String {
    if text.is_empty() {
        format!("{mention} ")
    } else if text.ends_with(char::is_whitespace) {
        format!("{text}{mention} ")
    } else {
        format!("{text} {mention} ")
    }
}

/// Scan composer text for `@`-mentions that resolve to existing **image**
/// files under `cwd` (PRODUCT §15b). These become attachment chips and are
/// sent as `image` content blocks. Non-image mentions stay plain text —
/// `claude` reads those itself. Order follows the text; duplicates collapse.
pub(super) fn image_mentions(text: &str, cwd: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for token in text.split_whitespace() {
        let Some(mention) = token.strip_prefix('@') else {
            continue;
        };
        if mention.is_empty() {
            continue;
        }
        let path = {
            let mentioned = Path::new(mention);
            if mentioned.is_absolute() {
                mentioned.to_path_buf()
            } else {
                cwd.join(mentioned)
            }
        };
        if image_media_type(&path).is_some() && path.is_file() && !found.contains(&path) {
            found.push(path);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leading_slash_is_a_slash_query_until_the_first_space() {
        let q = suggestion_query("/comp").expect("slash query");
        assert_eq!(q.kind, SuggestionKind::SlashCommand);
        assert_eq!(q.query, "comp");
        assert_eq!(q.token_range, 0..5);
        // Command committed; arguments are free text.
        assert_eq!(suggestion_query("/compact now"), None);
        // Bare slash suggests everything.
        assert_eq!(suggestion_query("/").expect("bare").query, "");
    }

    #[test]
    fn trailing_at_token_is_a_file_query() {
        let q = suggestion_query("look at @src/ma").expect("file query");
        assert_eq!(q.kind, SuggestionKind::FileMention);
        assert_eq!(q.query, "src/ma");
        assert_eq!(&"look at @src/ma"[q.token_range.clone()], "@src/ma");
        // Bare @ lists everything.
        assert_eq!(suggestion_query("see @").expect("bare").query, "");
        // No trailing @ token → no query.
        assert_eq!(suggestion_query("plain text"), None);
        assert_eq!(
            suggestion_query("a@b"),
            None,
            "mid-token @ is an email-ish, not a mention"
        );
    }

    #[test]
    fn trailing_slash_token_is_a_slash_query_anywhere() {
        // A `/` token mid-message opens the catalogue just like at column zero.
        let q = suggestion_query("say /comp").expect("slash query");
        assert_eq!(q.kind, SuggestionKind::SlashCommand);
        assert_eq!(q.query, "comp");
        assert_eq!(&"say /comp"[q.token_range.clone()], "/comp");
        // Accepting splices the command in at the token, leaving the prefix.
        assert_eq!(apply_suggestion("say /comp", &q, "compact"), "say /compact ");
    }

    #[test]
    fn apply_suggestion_replaces_the_token_and_commits_with_a_space() {
        let q = suggestion_query("/comp").unwrap();
        assert_eq!(apply_suggestion("/comp", &q, "compact"), "/compact ");
        let q = suggestion_query("look at @src/ma").unwrap();
        assert_eq!(
            apply_suggestion("look at @src/ma", &q, "src/main.rs"),
            "look at @src/main.rs "
        );
    }

    #[test]
    fn filter_ranks_fuzzy_matches_and_caps() {
        let candidates: Vec<String> = ["compact", "config", "cost", "clear", "doctor"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let filtered = filter_suggestions("co", &candidates);
        assert!(filtered.contains(&"compact".to_string()));
        assert!(!filtered.contains(&"clear".to_string()) || filtered.len() <= MAX_SUGGESTIONS);
        assert!(filter_suggestions("zzz", &candidates).is_empty());
        let empty = filter_suggestions("", &candidates);
        assert_eq!(empty.len(), candidates.len().min(MAX_SUGGESTIONS));
    }

    #[test]
    fn normalize_media_type_maps_aliases_and_rejects_others() {
        assert_eq!(normalize_image_media_type("image/png"), Some("image/png"));
        assert_eq!(normalize_image_media_type("image/jpg"), Some("image/jpeg"));
        assert_eq!(normalize_image_media_type("IMAGE/JPEG"), Some("image/jpeg"));
        assert_eq!(normalize_image_media_type("image/webp"), Some("image/webp"));
        assert_eq!(normalize_image_media_type("image/svg+xml"), None);
        assert_eq!(normalize_image_media_type("text/plain"), None);
    }

    #[test]
    fn mention_for_relativizes_under_cwd_and_keeps_absolute_outside() {
        let cwd = Path::new("/repo");
        assert_eq!(mention_for(Path::new("/repo/src/a.txt"), cwd), "@src/a.txt");
        assert_eq!(
            mention_for(Path::new("/elsewhere/b.txt"), cwd),
            "@/elsewhere/b.txt"
        );
    }

    #[test]
    fn append_mention_spaces_correctly() {
        assert_eq!(append_mention("", "@a.txt"), "@a.txt ");
        assert_eq!(append_mention("see ", "@a.txt"), "see @a.txt ");
        assert_eq!(append_mention("see", "@a.txt"), "see @a.txt ");
    }

    #[test]
    fn image_media_types_cover_the_api_set() {
        assert_eq!(image_media_type(Path::new("a.PNG")), Some("image/png"));
        assert_eq!(image_media_type(Path::new("b.jpeg")), Some("image/jpeg"));
        assert_eq!(image_media_type(Path::new("c.webp")), Some("image/webp"));
        assert_eq!(image_media_type(Path::new("d.rs")), None);
        assert_eq!(image_media_type(Path::new("noext")), None);
    }

    #[test]
    fn image_mentions_resolve_against_cwd_and_dedupe() {
        let dir = std::env::temp_dir().join("twarp-test-image-mentions");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("shot.png"), b"png").unwrap();
        std::fs::write(dir.join("notes.txt"), b"txt").unwrap();
        let text = "compare @shot.png with @notes.txt and @shot.png again, @missing.png too";
        let mentions = image_mentions(text, &dir);
        assert_eq!(mentions, vec![dir.join("shot.png")]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_cwd_files_walks_relative_and_capped() {
        let dir = std::env::temp_dir().join("twarp-test-cwd-files");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.rs"), b"x").unwrap();
        std::fs::write(dir.join("sub/b.rs"), b"x").unwrap();
        let files = list_cwd_files(&dir);
        assert_eq!(files, vec!["a.rs".to_string(), "sub/b.rs".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_frontmatter_description_reads_the_value() {
        let md = "---\nname: foo\ndescription: Does a thing. Use when X.\n---\n# Foo\nbody";
        assert_eq!(
            parse_frontmatter_description(md).as_deref(),
            Some("Does a thing. Use when X.")
        );
        // Quoted values are unquoted.
        let quoted = "---\ndescription: \"quoted desc\"\n---\n";
        assert_eq!(
            parse_frontmatter_description(quoted).as_deref(),
            Some("quoted desc")
        );
        // No frontmatter / no key → None.
        assert_eq!(parse_frontmatter_description("# just a heading"), None);
        assert_eq!(parse_frontmatter_description("---\nname: foo\n---\n"), None);
    }

    #[test]
    fn build_slash_command_index_discovers_project_skills_with_descriptions() {
        let dir = std::env::temp_dir().join("twarp-test-slash-index");
        let _ = std::fs::remove_dir_all(&dir);
        let skill_dir = dir.join(".claude").join("skills").join("my-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: my-skill\ndescription: My local skill.\n---\n# body",
        )
        .unwrap();

        let index = build_slash_command_index(&dir);
        // Built-ins are present...
        assert!(index.names.iter().any(|n| n == "compact"));
        // ...and so is the discovered project skill, with its description.
        assert!(index.names.iter().any(|n| n == "my-skill"));
        assert_eq!(
            index.descriptions.get("my-skill").map(String::as_str),
            Some("My local skill.")
        );
        // Names stay sorted/deduped.
        let mut sorted = index.names.clone();
        sorted.sort();
        assert_eq!(index.names, sorted);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn merge_names_adds_unseen_and_keeps_sorted() {
        let mut index = SlashCommandIndex::default();
        index.merge_names(["zebra", "alpha"]);
        index.merge_names(["alpha", "mango"]);
        assert_eq!(index.names, vec!["alpha", "mango", "zebra"]);
    }
}
