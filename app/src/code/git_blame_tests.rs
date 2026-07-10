use super::*;

const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const ZERO_SHA: &str = "0000000000000000000000000000000000000000";

fn committed(line: &Option<BlameLine>) -> &BlameCommitRef {
    match &line.as_ref().expect("line").state {
        BlameLineState::Committed(commit) => commit,
        BlameLineState::Uncommitted => panic!("expected committed line"),
    }
}

#[test]
fn parses_grouped_records_and_filenames_with_spaces() {
    let parsed = parse_git_blame_porcelain(&format!(
        "{SHA_A} 1 1 2\n\
         author Alice\n\
         author-mail <alice@example.com>\n\
         author-time 1700000000\n\
         author-tz -0800\n\
         summary initial commit\n\
         filename src/file with spaces.rs\n\
         \tfirst\n\
         \tsecond\n"
    ));

    assert_eq!(parsed.lines.len(), 2);
    assert_eq!(committed(&parsed.lines[0]).author_name, "Alice");
    assert_eq!(committed(&parsed.lines[1]).short_sha, "aaaaaaa");
    assert_eq!(
        committed(&parsed.lines[1]).original_filename.as_deref(),
        Some("src/file with spaces.rs")
    );
}

#[test]
fn reuses_commit_metadata_for_repeated_commits() {
    let parsed = parse_git_blame_porcelain(&format!(
        "{SHA_A} 1 1 1\n\
         author Alice\n\
         author-time 1700000000\n\
         filename src/lib.rs\n\
         \tone\n\
         {SHA_A} 2 2\n\
         \ttwo\n\
         {SHA_B} 3 3\n\
         author Bob\n\
         filename src/lib.rs\n\
         \tthree\n"
    ));

    assert_eq!(committed(&parsed.lines[1]).author_name, "Alice");
    assert_eq!(committed(&parsed.lines[2]).author_name, "Bob");
}

#[test]
fn treats_zero_sha_as_uncommitted() {
    let parsed = parse_git_blame_porcelain(&format!(
        "{ZERO_SHA} 1 1 1\n\
         author Not Committed Yet\n\
         filename src/lib.rs\n\
         \tdirty\n"
    ));

    assert!(matches!(
        parsed.lines[0].as_ref().expect("line").state,
        BlameLineState::Uncommitted
    ));
}

#[test]
fn tolerates_unknown_fields_and_missing_optional_fields() {
    let parsed = parse_git_blame_porcelain(&format!(
        "{SHA_A} 1 1 1\n\
         author Alice\n\
         previous deadbeef old.rs\n\
         filename src/lib.rs\n\
         \tone\n"
    ));

    assert_eq!(committed(&parsed.lines[0]).author_name, "Alice");
    assert_eq!(committed(&parsed.lines[0]).author_email, None);
}

#[test]
fn reports_malformed_trailing_records_without_failing_parse() {
    let parsed = parse_git_blame_porcelain(&format!(
        "{SHA_A} 1 1 1\n\
         author Alice\n\
         filename src/lib.rs\n"
    ));

    assert!(parsed.lines.is_empty());
    assert!(!parsed.diagnostics.is_empty());
}

#[test]
fn parses_commit_detail_metadata_with_body() {
    let parsed = parse_git_show_commit_metadata(&format!(
        "{SHA_B}\0Bob\0bob@example.com\02024-01-02T03:04:05+00:00\0subject\n\nbody line"
    ))
    .expect("metadata parses");

    assert_eq!(parsed.full_sha, SHA_B);
    assert_eq!(parsed.author_name, "Bob");
    assert_eq!(parsed.author_email.as_deref(), Some("bob@example.com"));
    assert_eq!(
        parsed.absolute_author_date.as_deref(),
        Some("2024-01-02 03:04:05 +00:00")
    );
    assert_eq!(parsed.message, "subject\n\nbody line");
}

#[test]
fn builds_github_commit_url_from_origin() {
    assert_eq!(
        github_commit_url("git@github.com:owner/repo.git", SHA_A).as_deref(),
        Some("https://github.com/owner/repo/commit/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    );
    assert_eq!(
        github_commit_url("https://gitlab.com/owner/repo.git", SHA_A),
        None
    );
}
