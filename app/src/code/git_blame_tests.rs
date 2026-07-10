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
