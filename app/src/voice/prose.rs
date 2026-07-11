//! Markdown → speakable prose (twarp 17, PRODUCT §13).
//!
//! Spoken replies read the prose and skip everything that doesn't work as
//! speech: fenced code blocks, tables, and markdown syntax. Operates on the
//! turn's raw markdown text (the same string `last_assistant_text` yields), so
//! it needs no hooks into the rendered transcript.

/// Strip `markdown` down to plain speakable prose. Fenced code blocks and
/// table rows are dropped entirely; inline markup keeps its text content.
pub fn markdown_to_prose(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut in_fence = false;
    for line in markdown.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        // Tables and horizontal rules don't speak.
        if trimmed.starts_with('|') || is_horizontal_rule(trimmed) {
            continue;
        }
        let line = strip_line_markup(trimmed);
        let line = line.trim();
        if line.is_empty() {
            // Preserve paragraph breaks as sentence pauses.
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            continue;
        }
        if !out.is_empty() && !out.ends_with('\n') {
            out.push(' ');
        }
        out.push_str(line);
    }
    out.trim().to_owned()
}

fn is_horizontal_rule(line: &str) -> bool {
    let bare: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    bare.len() >= 3 && (bare.chars().all(|c| c == '-') || bare.chars().all(|c| c == '*'))
}

/// Drop block prefixes (headings, quotes, list markers) and inline markup
/// (emphasis, inline code, links keep their text).
fn strip_line_markup(line: &str) -> String {
    let line = line.trim_start_matches('#').trim_start();
    let line = line.strip_prefix('>').map(str::trim_start).unwrap_or(line);
    let line = line
        .strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .or_else(|| line.strip_prefix("+ "))
        .unwrap_or(line);

    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' | '_' | '`' => {}
            // [text](url) → text
            '[' => {
                let mut text = String::new();
                let mut closed = false;
                for inner in chars.by_ref() {
                    if inner == ']' {
                        closed = true;
                        break;
                    }
                    text.push(inner);
                }
                out.push_str(&text);
                if closed && chars.peek() == Some(&'(') {
                    chars.next();
                    for inner in chars.by_ref() {
                        if inner == ')' {
                            break;
                        }
                    }
                }
            }
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_code_fences_keeps_prose() {
        let md = "Here is the fix:\n\n```rust\nfn main() {}\n```\n\nRun it with cargo.";
        assert_eq!(
            markdown_to_prose(md),
            "Here is the fix:\nRun it with cargo."
        );
    }

    #[test]
    fn strips_inline_markup() {
        assert_eq!(
            markdown_to_prose("Use **bold** and `code` and [a link](https://x.dev)."),
            "Use bold and code and a link."
        );
    }

    #[test]
    fn drops_tables_and_rules() {
        let md = "Results:\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n---\n\nDone.";
        assert_eq!(markdown_to_prose(md), "Results:\nDone.");
    }

    #[test]
    fn strips_headings_quotes_lists() {
        let md = "## Plan\n> note\n- first\n* second";
        assert_eq!(markdown_to_prose(md), "Plan note first second");
    }

    #[test]
    fn empty_and_code_only_yield_empty() {
        assert_eq!(markdown_to_prose("```\nonly code\n```"), "");
        assert_eq!(markdown_to_prose("   "), "");
    }
}
