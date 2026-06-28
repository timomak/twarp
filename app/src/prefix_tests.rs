use crate::prefix::longest_common_prefix;

#[test]
fn test_basic_prefix() {
    let strings = ["foo", "foobar", "foofoobar"];

    let result = longest_common_prefix(strings);
    assert_eq!(result.unwrap(), "foo");
}

#[test]
fn test_no_prefix() {
    let strings = ["foo", "foobar", "bar"];
    let result = longest_common_prefix(strings);
    assert_eq!(result, None);
}

#[test]
fn test_single_string() {
    let strings = ["foo"];
    let result = longest_common_prefix(strings);
    assert_eq!(result.unwrap(), "foo");
}

#[test]
fn test_no_string() {
    let strings = [];
    let result = longest_common_prefix(strings);
    assert_eq!(result, None);
}

#[test]
fn test_multibyte_strings() {
    let strings = ["ay東cz", "ay東ab"];
    let result = longest_common_prefix(strings);
    assert_eq!(result.unwrap(), "ay東");
}

#[test]
fn test_multibyte_strings_common_bytes() {
    let strings = ["ab東早", "ab東旪"];
    let result = longest_common_prefix(strings);
    assert_eq!(result.unwrap(), "ab東");
}

#[test]
fn test_identical_strings() {
    let strings = ["foo東", "foo東", "foo東"];
    let result = longest_common_prefix(strings);
    assert_eq!(result.unwrap(), "foo東");
}

#[test]
fn test_prefix_ends_at_utf8_boundary() {
    // 'é' (U+00E9 = 0xC3 0xA9) and 'ç' (U+00E7 = 0xC3 0xA7) share their leading
    // byte. A byte-wise prefix would stop at "caf\u{C3}", which is not a valid
    // UTF-8 boundary and would panic when sliced, so the prefix must land on a
    // char boundary: "caf".
    let strings = ["caf\u{E9}", "caf\u{E7}"];
    let result = longest_common_prefix(strings);
    assert_eq!(result.unwrap(), "caf");
}

#[test]
fn test_contains_empty_string() {
    // An empty string shares no prefix with anything, so the result is None.
    let strings = ["foo", "", "foobar"];
    let result = longest_common_prefix(strings);
    assert_eq!(result, None);
}

#[test]
fn test_first_is_empty_string() {
    let strings = ["", "foo"];
    let result = longest_common_prefix(strings);
    assert_eq!(result, None);
}
