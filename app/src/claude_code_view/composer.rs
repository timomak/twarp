//! Composer intelligence for the Claude Code pane (feature 07, sub-phase 7j;
//! PRODUCT §15a–§15b):
//!
//! - **`/` suggestions** — typing a leading `/` filters the session's slash
//!   commands (the `init` message's `slash_commands`, plus a small built-in
//!   set so a fresh pane isn't empty before the first init).
//! - **`@` file mentions** — an `@`-prefixed token fuzzy-filters the cwd's
//!   files (gitignore-aware walk, capped). Accepting inserts the relative
//!   path; `claude` reads mentioned files itself.
//! - **Image attachments** — `@`-mentions that resolve to image files under
//!   the cwd become real attachments: previewed as chips above the composer
//!   and sent as base64 `image` content blocks (verified accepted by
//!   `claude`'s stream-json input), so the model *sees* the image without a
//!   tool round-trip.
//!
//! The pure parsing/filtering/scanning lives here, unit-tested headlessly;
//! the view owns the state and rendering.

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

/// Cap on files gathered by the cwd walk — enough for real repos, bounded so
/// a giant tree can't stall the composer.
pub(super) const MAX_CWD_FILES: usize = 5_000;

/// Hard ceiling for one attached image's raw size. The API rejects ~5 MB of
/// base64; 3.75 MB raw encodes right up to that.
pub(super) const MAX_IMAGE_BYTES: u64 = 3_750_000;

/// Derive the active suggestion query from the composer text (PRODUCT §15a).
///
/// - A leading `/` with no whitespace yet is a slash-command query (slash
///   commands are message-initial in `claude`; once a space is typed the
///   command is committed and arguments are free text).
/// - Otherwise, a trailing `@`-prefixed token is a file query.
pub(super) fn suggestion_query(text: &str) -> Option<SuggestionQuery> {
    if let Some(rest) = text.strip_prefix('/') {
        if !rest.contains(char::is_whitespace) {
            return Some(SuggestionQuery {
                kind: SuggestionKind::SlashCommand,
                query: rest.to_owned(),
                token_range: 0..text.len(),
            });
        }
        return None;
    }
    let token_start = text
        .rfind(char::is_whitespace)
        .map(|idx| idx + text[idx..].chars().next().map_or(1, char::len_utf8))
        .unwrap_or(0);
    let token = &text[token_start..];
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

/// Walk the cwd for mentionable files: gitignore-aware, relative paths,
/// files only, capped at [`MAX_CWD_FILES`] (PRODUCT §15a).
pub(super) fn list_cwd_files(cwd: &Path) -> Vec<String> {
    let mut files = Vec::new();
    for entry in ignore::WalkBuilder::new(cwd)
        .hidden(true)
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
    fn slash_only_counts_at_message_start() {
        assert_eq!(suggestion_query("say /compact"), None);
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
}
