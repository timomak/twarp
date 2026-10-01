use super::existing_path;
use std::fs::{File, FileTimes};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

fn transcript(root: &Path, name: &str, text: &str, modified: u64) -> PathBuf {
    let path = root.join(name);
    std::fs::write(
        &path,
        serde_json::json!({"type": "user", "message": {"role": "user", "content": text}})
            .to_string(),
    )
    .unwrap();
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(
            FileTimes::new().set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(modified)),
        )
        .unwrap();
    path
}

#[test]
fn fresh_mapped_history_is_found_when_original_has_no_transcript() {
    let directory = tempfile::tempdir().unwrap();
    let mapped = transcript(directory.path(), "mapped.jsonl", "mapped turn", 10);
    let selected = existing_path(
        None,
        Some(directory.path().join("missing-original.jsonl")),
        Some(mapped.clone()),
    )
    .unwrap();
    assert_eq!(selected, mapped);
    assert!(matches!(
        &claude_code::sessions::load_history(&selected)[0],
        claude_code::TranscriptEvent::UserMessage(text) if text == "mapped turn"
    ));
}

#[test]
fn resumed_original_history_stays_pinned_after_repeated_checkout_relocations() {
    let directory = tempfile::tempdir().unwrap();
    let original = transcript(directory.path(), "original.jsonl", "original turn", 10);
    let first = transcript(directory.path(), "first-mapping.jsonl", "first mapping", 20);
    let second = transcript(
        directory.path(),
        "second-mapping.jsonl",
        "second mapping",
        30,
    );
    let before = std::fs::read(&original).unwrap();
    assert_eq!(
        existing_path(Some(&original), Some(original.clone()), Some(first)),
        Some(original.clone())
    );
    assert_eq!(
        existing_path(Some(&original), Some(original.clone()), Some(second)),
        Some(original.clone())
    );
    assert_eq!(std::fs::read(original).unwrap(), before);
}

#[test]
fn fresh_session_keeps_previous_mapped_history_when_the_checkout_is_mapped_again() {
    let directory = tempfile::tempdir().unwrap();
    let first = transcript(directory.path(), "first-mapping.jsonl", "existing chat", 10);
    let second = transcript(directory.path(), "second-mapping.jsonl", "other copy", 20);
    assert_eq!(
        existing_path(
            Some(&first),
            Some(directory.path().join("missing-original.jsonl")),
            Some(second)
        ),
        Some(first)
    );
}

#[test]
fn legacy_snapshot_without_a_pinned_path_uses_the_newer_explicit_location() {
    let directory = tempfile::tempdir().unwrap();
    let original = transcript(directory.path(), "original.jsonl", "old turn", 10);
    let mapped = transcript(directory.path(), "mapped.jsonl", "new turn", 20);
    assert_eq!(
        existing_path(None, Some(original), Some(mapped.clone())),
        Some(mapped)
    );
}

#[test]
fn missing_recorded_path_falls_back_to_existing_original_history() {
    let directory = tempfile::tempdir().unwrap();
    let original = transcript(directory.path(), "original.jsonl", "kept turn", 10);
    assert_eq!(
        existing_path(
            Some(&directory.path().join("other-machine.jsonl")),
            Some(original.clone()),
            Some(directory.path().join("missing-mapped.jsonl"))
        ),
        Some(original)
    );
}

#[test]
fn unwritten_spawn_location_does_not_hide_a_later_mapped_transcript() {
    let directory = tempfile::tempdir().unwrap();
    let mapped = transcript(directory.path(), "later-mapping.jsonl", "first turn", 10);
    assert_eq!(
        existing_path(
            Some(&directory.path().join("unwritten-spawn.jsonl")),
            Some(directory.path().join("missing-original.jsonl")),
            Some(mapped.clone())
        ),
        Some(mapped)
    );
}
