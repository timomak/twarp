//! twarp 05b: synthesize a single-hunk unified-diff patch suitable for
//! `git apply --cached` / `git apply --cached --reverse`.
//!
//! Stage / unstage hunk in PRODUCT.md §12 routes a click on a hunk's
//! `[+]` / `[−]` affordance through this module:
//!
//! 1. The hunk header line range is forwarded to the parent view.
//! 2. The parent resolves the matching [`DiffHunk`] in the file's
//!    `FileDiff::hunks`.
//! 3. [`hunk_to_patch`] builds the patch text fed to `git apply`.
//!
//! Output format mirrors `git diff` so `git apply --check` round-trips:
//! ```text
//! diff --git a/<path> b/<path>
//! --- a/<path>
//! +++ b/<path>
//! @@ -<old_start>,<old_count> +<new_start>,<new_count> @@
//!  <context lines>
//! -<deleted lines>
//! +<added lines>
//! ```
//!
//! For added or deleted files, the `---` / `+++` headers use `/dev/null`
//! on the missing side, matching git's own behavior. The synthesized
//! patch always contains exactly one hunk — partial stages of multiple
//! hunks are driven by multiple invocations.

use std::path::Path;

use crate::code_review::diff_state::{DiffHunk, DiffLine, DiffLineType, GitFileStatus};

const NO_NEWLINE_MARKER: &str = "\\ No newline at end of file";

/// Synthesize a single-hunk unified-diff patch for the given file.
///
/// `path` is the repo-relative path. `status` selects the file
/// headers (`/dev/null` vs `a/<path>` / `b/<path>`). `hunk` provides
/// the content; its `lines` are streamed verbatim with a one-character
/// prefix (`' '`, `'-'`, or `'+'`) per [`DiffLineType`]. `HunkHeader`
/// entries inside `hunk.lines` are skipped — the header is regenerated
/// from the hunk's `old_start_line` / `new_start_line` fields so we
/// don't rely on the parser having preserved a roundtrip-equal header.
///
/// The result always ends with a trailing newline so `git apply` reads
/// the final line correctly, except when the last hunk line is marked
/// `no_trailing_newline`, in which case the `\ No newline at end of
/// file` marker is appended without a trailing newline.
pub fn hunk_to_patch(path: &Path, status: GitFileStatus, hunk: &DiffHunk) -> String {
    let path_str = path.to_string_lossy();
    let mut out = String::with_capacity(estimate_capacity(&path_str, hunk));

    out.push_str("diff --git a/");
    out.push_str(&path_str);
    out.push_str(" b/");
    out.push_str(&path_str);
    out.push('\n');

    let (a_path, b_path) = match status {
        GitFileStatus::New | GitFileStatus::Untracked => {
            ("/dev/null".to_string(), format!("b/{path_str}"))
        }
        GitFileStatus::Deleted => (format!("a/{path_str}"), "/dev/null".to_string()),
        _ => (format!("a/{path_str}"), format!("b/{path_str}")),
    };
    out.push_str("--- ");
    out.push_str(&a_path);
    out.push('\n');
    out.push_str("+++ ");
    out.push_str(&b_path);
    out.push('\n');

    out.push_str(&format_hunk_header(hunk));
    out.push('\n');

    for line in hunk
        .lines
        .iter()
        .filter(|l| l.line_type != DiffLineType::HunkHeader)
    {
        push_line(&mut out, line);
    }

    out
}

fn format_hunk_header(hunk: &DiffHunk) -> String {
    format!(
        "@@ -{} +{} @@",
        format_range(hunk.old_start_line, hunk.old_line_count),
        format_range(hunk.new_start_line, hunk.new_line_count),
    )
}

fn format_range(start: usize, count: usize) -> String {
    if count == 1 {
        format!("{start}")
    } else {
        format!("{start},{count}")
    }
}

fn push_line(out: &mut String, line: &DiffLine) {
    let prefix = match line.line_type {
        DiffLineType::Context => ' ',
        DiffLineType::Add => '+',
        DiffLineType::Delete => '-',
        DiffLineType::HunkHeader => return,
    };
    out.push(prefix);
    out.push_str(&line.text);
    if line.no_trailing_newline {
        out.push('\n');
        out.push_str(NO_NEWLINE_MARKER);
    } else {
        out.push('\n');
    }
}

fn estimate_capacity(path_str: &str, hunk: &DiffHunk) -> usize {
    let header_overhead = 4 * path_str.len() + 64;
    let line_overhead: usize = hunk.lines.iter().map(|l| l.text.len() + 2).sum();
    header_overhead + line_overhead
}

#[cfg(test)]
#[path = "hunk_patch_tests.rs"]
mod tests;
