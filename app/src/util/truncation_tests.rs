use super::*;

#[test]
fn truncate_from_end_returns_empty_string_unchanged() {
    assert_eq!(truncate_from_end("", 0), "");
    assert_eq!(truncate_from_end("", 10), "");
}

#[test]
fn truncate_from_beginning_returns_empty_string_unchanged() {
    assert_eq!(truncate_from_beginning("", 0), "");
    assert_eq!(truncate_from_beginning("", 10), "");
}

#[test]
fn truncate_from_end_with_zero_max_length_returns_ellipsis_for_non_empty_text() {
    assert_eq!(truncate_from_end("hello", 0), "…");
}

#[test]
fn truncate_from_beginning_with_zero_max_length_returns_ellipsis_for_non_empty_text() {
    assert_eq!(truncate_from_beginning("hello", 0), "…");
}

#[test]
fn truncate_from_end_returns_shorter_text_unchanged() {
    assert_eq!(truncate_from_end("hello", 10), "hello");
}

#[test]
fn truncate_from_beginning_returns_shorter_text_unchanged() {
    assert_eq!(truncate_from_beginning("hello", 10), "hello");
}

#[test]
fn truncate_from_end_returns_exact_length_text_unchanged() {
    assert_eq!(truncate_from_end("hello", 5), "hello");
}

#[test]
fn truncate_from_beginning_returns_exact_length_text_unchanged() {
    assert_eq!(truncate_from_beginning("hello", 5), "hello");
}

#[test]
fn truncate_from_end_replaces_trailing_text_with_ellipsis() {
    assert_eq!(truncate_from_end("hello world", 8), "hello w…");
}

#[test]
fn truncate_from_beginning_replaces_leading_text_with_ellipsis() {
    assert_eq!(truncate_from_beginning("hello world", 8), "…o world");
}

#[test]
fn truncate_from_end_handles_multibyte_unicode_near_boundary() {
    assert_eq!(truncate_from_end("café🚀test", 6), "café🚀…");
}

#[test]
fn truncate_from_beginning_handles_multibyte_unicode_near_boundary() {
    assert_eq!(truncate_from_beginning("café🚀test", 6), "…🚀test");
}
