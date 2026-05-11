//! Unit tests for the custom-shortcuts runtime (4a).
//!
//! These cover the pure-data slice of the feature: YAML parsing, error
//! messages (PRODUCT §20 — every row asserted verbatim against manufactured
//! malformed YAML, since the messages are part of the user-facing contract),
//! and the `KeyName` → PTY byte table.
//!
//! Executor / dispatch invariants that require a `Workspace` view (PRODUCT
//! §§6, 7, 8, 12, 13, 14, 16, 17) rely on the smoke test in PRODUCT.md
//! since the keymap-routing layer is config-shaped and hard to fake.

use std::time::Duration;

use crate::pane_group::Direction;
use crate::shortcuts::action::{Action, KeyName};
use crate::shortcuts::config::parse_shortcuts_yaml;
use crate::shortcuts::key_to_bytes::bytes_for;

#[track_caller]
fn assert_error(yaml: &str, expected: &str) {
    let result = parse_shortcuts_yaml(yaml);
    assert!(
        result.errors.iter().any(|e| e == expected),
        "expected error not found.\nexpected: {expected}\ngot errors: {:#?}",
        result.errors
    );
}

#[track_caller]
fn assert_no_errors(yaml: &str) {
    let result = parse_shortcuts_yaml(yaml);
    assert!(
        result.errors.is_empty(),
        "expected no errors, got: {:#?}",
        result.errors
    );
}

// --- Empty / missing input (PRODUCT §22) ---

#[test]
fn empty_input_yields_zero_shortcuts() {
    let result = parse_shortcuts_yaml("");
    assert!(result.shortcuts.is_empty());
    assert!(result.errors.is_empty());
}

#[test]
fn null_shortcuts_yields_zero() {
    let result = parse_shortcuts_yaml("shortcuts:\n");
    assert!(result.shortcuts.is_empty());
    assert!(result.errors.is_empty());
}

#[test]
fn empty_list_yields_zero() {
    let result = parse_shortcuts_yaml("shortcuts: []\n");
    assert!(result.shortcuts.is_empty());
    assert!(result.errors.is_empty());
}

// --- §21: YAML parse errors ---

#[test]
fn unparseable_yaml_yields_single_parse_error() {
    let result = parse_shortcuts_yaml("shortcuts:\n  - keys: \"unterminated");
    assert_eq!(result.shortcuts.len(), 0);
    assert_eq!(result.errors.len(), 1, "errors: {:?}", result.errors);
    assert!(
        result.errors[0].starts_with("shortcuts.yaml: failed to parse: "),
        "got: {}",
        result.errors[0]
    );
}

// --- §20 row "Top-level is not a `shortcuts:` map" ---

#[test]
fn top_level_list_is_error() {
    assert_error(
        "- keys: cmdorctrl-1\n  actions: [new_tab]\n",
        "shortcuts.yaml: expected top-level 'shortcuts:' key with a list value",
    );
}

#[test]
fn top_level_scalar_is_error() {
    assert_error(
        "hello\n",
        "shortcuts.yaml: expected top-level 'shortcuts:' key with a list value",
    );
}

#[test]
fn top_level_unexpected_key_is_error() {
    assert_error(
        "other_key: foo\n",
        "shortcuts.yaml: expected top-level 'shortcuts:' key with a list value",
    );
}

// --- §20 row "`shortcuts:` value is not a list" ---

#[test]
fn shortcuts_value_string_is_error() {
    assert_error(
        "shortcuts: hello\n",
        "shortcuts.yaml: 'shortcuts:' must be a list, got string",
    );
}

#[test]
fn shortcuts_value_map_is_error() {
    assert_error(
        "shortcuts:\n  foo: bar\n",
        "shortcuts.yaml: 'shortcuts:' must be a list, got map",
    );
}

// --- §20 row "Entry missing `keys`" ---

#[test]
fn entry_missing_keys_is_error() {
    assert_error(
        "shortcuts:\n  - actions: [new_tab]\n",
        "shortcuts.yaml: entry #1: missing required field 'keys'",
    );
}

// --- §20 row "Entry missing or empty `actions`" ---

#[test]
fn entry_missing_actions_is_error() {
    assert_error(
        "shortcuts:\n  - keys: cmdorctrl-1\n",
        "shortcuts.yaml: entry #1 ('cmd-1'): missing required field 'actions' (must have at least one action)",
    );
}

#[test]
fn entry_empty_actions_is_error() {
    assert_error(
        "shortcuts:\n  - keys: cmdorctrl-1\n    actions: []\n",
        "shortcuts.yaml: entry #1 ('cmd-1'): missing required field 'actions' (must have at least one action)",
    );
}

// --- §20 row "Unknown field on entry" ---

#[test]
fn entry_unknown_field_is_error() {
    // Pre-parse errors (before the chord round-trips through `Keystroke::parse`)
    // surface the raw `keys` value the user typed. Pick a chord that
    // round-trips unchanged so the test is platform-agnostic.
    assert_error(
        "shortcuts:\n  - keys: cmd-1\n    actions: [new_tab]\n    surprise: yes\n",
        "shortcuts.yaml: entry #1 ('cmd-1'): unknown field 'surprise'; expected 'keys' and 'actions'",
    );
}

// --- §20 row "Malformed key chord" ---

#[test]
fn malformed_chord_is_error() {
    assert_error(
        "shortcuts:\n  - keys: \"cmd+shift+d\"\n    actions: [new_tab]\n",
        "shortcuts.yaml: entry #1: invalid key chord 'cmd+shift+d'; expected modifiers (cmdorctrl/cmd/ctrl/alt/shift/meta) joined by '-' with a key, e.g. 'cmdorctrl-shift-D'",
    );
}

// --- §20 row "Action is not a string or single-key map" ---

#[test]
fn action_multi_key_map_is_error() {
    assert_error(
        "shortcuts:\n  - keys: cmdorctrl-1\n    actions:\n      - type: hi\n        press: enter\n",
        "shortcuts.yaml: entry #1 ('cmd-1'), action #1: expected a bare action name or a single-key map",
    );
}

#[test]
fn action_bare_type_when_value_needed_is_error() {
    // `type` requires a value; using it as a bare string is not allowed.
    assert_error(
        "shortcuts:\n  - keys: cmdorctrl-1\n    actions:\n      - type\n",
        "shortcuts.yaml: entry #1 ('cmd-1'), action #1: expected a bare action name or a single-key map",
    );
}

// --- §20 row "Unknown action token" ---

#[test]
fn unknown_action_token_is_error() {
    assert_error(
        "shortcuts:\n  - keys: cmdorctrl-1\n    actions:\n      - frobnicate\n",
        "shortcuts.yaml: entry #1 ('cmd-1'), action #1: unknown action 'frobnicate'; expected one of new_tab, new_pane, type, press, wait",
    );
}

#[test]
fn unknown_action_token_in_map_is_error() {
    assert_error(
        "shortcuts:\n  - keys: cmdorctrl-1\n    actions:\n      - frobnicate: please\n",
        "shortcuts.yaml: entry #1 ('cmd-1'), action #1: unknown action 'frobnicate'; expected one of new_tab, new_pane, type, press, wait",
    );
}

// --- §20 row "Missing `new_pane` direction" ---

#[test]
fn new_pane_missing_direction_is_error() {
    // Bare `new_pane` (no value) — same error as null value.
    assert_error(
        "shortcuts:\n  - keys: cmdorctrl-1\n    actions:\n      - new_pane\n",
        "shortcuts.yaml: entry #1 ('cmd-1'), action #1: 'new_pane' requires a direction; expected 'right' or 'down'",
    );
}

#[test]
fn new_pane_null_direction_is_error() {
    assert_error(
        "shortcuts:\n  - keys: cmdorctrl-1\n    actions:\n      - new_pane:\n",
        "shortcuts.yaml: entry #1 ('cmd-1'), action #1: 'new_pane' requires a direction; expected 'right' or 'down'",
    );
}

// --- §20 row "Invalid `new_pane` direction" ---

#[test]
fn new_pane_invalid_direction_is_error() {
    assert_error(
        "shortcuts:\n  - keys: cmdorctrl-1\n    actions:\n      - new_pane: sideways\n",
        "shortcuts.yaml: entry #1 ('cmd-1'), action #1: invalid 'new_pane' direction 'sideways'; expected 'right' or 'down'",
    );
}

// --- §20 row "`type` value is not a string" ---

#[test]
fn type_non_string_is_error() {
    assert_error(
        "shortcuts:\n  - keys: cmdorctrl-1\n    actions:\n      - type: [a, b]\n",
        "shortcuts.yaml: entry #1 ('cmd-1'), action #1: 'type' expects a string value",
    );
}

// --- §20 row "Newline in `type`" ---

#[test]
fn type_newline_is_error() {
    assert_error(
        "shortcuts:\n  - keys: cmdorctrl-1\n    actions:\n      - type: \"a\\nb\"\n",
        "shortcuts.yaml: entry #1 ('cmd-1'), action #1: 'type' value contains a newline; use 'press: enter' to submit input",
    );
}

// --- §20 row "Unknown press key" ---

#[test]
fn press_unknown_key_is_error() {
    assert_error(
        "shortcuts:\n  - keys: cmdorctrl-1\n    actions:\n      - press: zzz\n",
        "shortcuts.yaml: entry #1 ('cmd-1'), action #1: unknown key 'zzz' in 'press'; expected one of enter, tab, escape, backspace, space, up, down, left, right, home, end, pageup, pagedown, delete, insert, numpadenter, f1-f12",
    );
}

// --- §20 row "Modifier in `press`" ---

#[test]
fn press_modifier_is_error() {
    assert_error(
        "shortcuts:\n  - keys: cmdorctrl-1\n    actions:\n      - press: ctrl-c\n",
        "shortcuts.yaml: entry #1 ('cmd-1'), action #1: 'press' does not support modifiers in v1 (got 'ctrl-c')",
    );
}

// --- §20 row "Invalid `wait` value" ---

#[test]
fn wait_garbage_is_error() {
    assert_error(
        "shortcuts:\n  - keys: cmdorctrl-1\n    actions:\n      - wait: \"2x\"\n",
        "shortcuts.yaml: entry #1 ('cmd-1'), action #1: invalid 'wait' value '2x'; expected a duration like '500ms', '2s', '1m' (1ms–60s)",
    );
}

#[test]
fn wait_zero_is_error() {
    assert_error(
        "shortcuts:\n  - keys: cmdorctrl-1\n    actions:\n      - wait: 0ms\n",
        "shortcuts.yaml: entry #1 ('cmd-1'), action #1: invalid 'wait' value '0ms'; expected a duration like '500ms', '2s', '1m' (1ms–60s)",
    );
}

#[test]
fn wait_too_long_is_error() {
    assert_error(
        "shortcuts:\n  - keys: cmdorctrl-1\n    actions:\n      - wait: 61s\n",
        "shortcuts.yaml: entry #1 ('cmd-1'), action #1: invalid 'wait' value '61s'; expected a duration like '500ms', '2s', '1m' (1ms–60s)",
    );
}

// --- §20 row "Duplicate `keys` (later wins)" ---

#[test]
fn duplicate_keys_keeps_last_with_error() {
    let yaml = r#"
shortcuts:
  - keys: cmdorctrl-1
    actions:
      - type: "first"
  - keys: cmdorctrl-1
    actions:
      - type: "second"
"#;
    let result = parse_shortcuts_yaml(yaml);
    assert_eq!(result.shortcuts.len(), 1);
    match &result.shortcuts[0].actions[0] {
        Action::Type(s) => assert_eq!(s, "second"),
        other => panic!("expected Type, got {other:?}"),
    }
    assert!(
        result.errors.iter().any(|e| e
            == "shortcuts.yaml: entry #2 ('cmd-1') is a duplicate of an earlier entry; using the last definition"),
        "errors: {:?}",
        result.errors
    );
}

// --- Default shortcuts.yaml content (PRODUCT §1 bootstrap) ---

#[test]
fn default_shortcuts_yaml_parses_to_the_two_driving_examples() {
    let result = parse_shortcuts_yaml(crate::shortcuts::DEFAULT_SHORTCUTS_YAML);
    assert!(result.errors.is_empty(), "errors: {:#?}", result.errors);
    assert_eq!(result.shortcuts.len(), 2);
    // First entry is cmd-shift-D running `claude`.
    assert!(matches!(
        result.shortcuts[0].actions[0],
        Action::NewPane(Direction::Right)
    ));
    assert!(matches!(
        result.shortcuts[0].actions[1],
        Action::Type(ref t) if t == "claude"
    ));
    // Second entry has the 3s wait + slash command.
    assert_eq!(result.shortcuts[1].actions.len(), 6);
    assert!(matches!(
        result.shortcuts[1].actions[3],
        Action::Wait(d) if d == Duration::from_secs(3)
    ));
    assert!(matches!(
        result.shortcuts[1].actions[4],
        Action::Type(ref t) if t == "/address-code-review-comments"
    ));
}

// --- Driving examples (PRODUCT §Driving examples) ---

#[test]
fn driving_examples_parse_without_errors() {
    let yaml = r#"
shortcuts:
  - keys: cmdorctrl-shift-D
    actions:
      - new_pane: right
      - type: "claude"
      - press: enter

  - keys: cmdorctrl-shift-A
    actions:
      - new_pane: right
      - type: "claude"
      - press: enter
      - wait: 3s
      - type: "/address-code-review-comments"
      - press: enter
"#;
    let result = parse_shortcuts_yaml(yaml);
    assert!(result.errors.is_empty(), "errors: {:#?}", result.errors);
    assert_eq!(result.shortcuts.len(), 2);

    assert_eq!(result.shortcuts[0].actions.len(), 3);
    assert!(matches!(
        result.shortcuts[0].actions[0],
        Action::NewPane(Direction::Right)
    ));
    assert!(matches!(
        result.shortcuts[0].actions[1],
        Action::Type(ref t) if t == "claude"
    ));
    assert!(matches!(
        result.shortcuts[0].actions[2],
        Action::Press(KeyName::Enter)
    ));

    assert_eq!(result.shortcuts[1].actions.len(), 6);
    assert!(matches!(
        result.shortcuts[1].actions[3],
        Action::Wait(d) if d == Duration::from_secs(3)
    ));
}

// --- Casing normalization (PRODUCT §4) ---

#[test]
fn shift_lowercase_is_normalized_to_uppercase() {
    let yaml = "shortcuts:\n  - keys: cmd-shift-d\n    actions: [new_tab]\n";
    assert_no_errors(yaml);
}

// --- Bare new_tab without parameter (PRODUCT §5) ---

#[test]
fn bare_new_tab_parses() {
    let yaml = "shortcuts:\n  - keys: cmdorctrl-1\n    actions:\n      - new_tab\n";
    let result = parse_shortcuts_yaml(yaml);
    assert!(result.errors.is_empty(), "errors: {:?}", result.errors);
    assert_eq!(result.shortcuts.len(), 1);
    assert!(matches!(result.shortcuts[0].actions[0], Action::NewTab));
}

// --- Invalid entry is skipped, valid ones load (PRODUCT §18) ---

#[test]
fn one_bad_entry_does_not_disable_the_file() {
    let yaml = r#"
shortcuts:
  - keys: cmdorctrl-1
    actions: [new_tab]
  - keys: cmdorctrl-2
    actions:
      - frobnicate
  - keys: cmdorctrl-3
    actions: [new_tab]
"#;
    let result = parse_shortcuts_yaml(yaml);
    assert_eq!(result.shortcuts.len(), 2);
    assert_eq!(result.errors.len(), 1);
}

// --- key_to_bytes table coverage (PRODUCT §10) ---

#[test]
fn key_to_bytes_table() {
    // One concrete byte per named key, verifying both the table and that the
    // mapping doesn't drift from PRODUCT §10's wire contract.
    assert_eq!(bytes_for(KeyName::Enter), b"\r");
    assert_eq!(bytes_for(KeyName::NumpadEnter), b"\r");
    assert_eq!(bytes_for(KeyName::Tab), b"\t");
    assert_eq!(bytes_for(KeyName::Escape), b"\x1b");
    assert_eq!(bytes_for(KeyName::Backspace), b"\x7f");
    assert_eq!(bytes_for(KeyName::Space), b" ");
    assert_eq!(bytes_for(KeyName::Up), b"\x1b[A");
    assert_eq!(bytes_for(KeyName::Down), b"\x1b[B");
    assert_eq!(bytes_for(KeyName::Right), b"\x1b[C");
    assert_eq!(bytes_for(KeyName::Left), b"\x1b[D");
    assert_eq!(bytes_for(KeyName::Home), b"\x1b[H");
    assert_eq!(bytes_for(KeyName::End), b"\x1b[F");
    assert_eq!(bytes_for(KeyName::PageUp), b"\x1b[5~");
    assert_eq!(bytes_for(KeyName::PageDown), b"\x1b[6~");
    assert_eq!(bytes_for(KeyName::Insert), b"\x1b[2~");
    assert_eq!(bytes_for(KeyName::Delete), b"\x1b[3~");
    assert_eq!(bytes_for(KeyName::F(1)), b"\x1bOP");
    assert_eq!(bytes_for(KeyName::F(4)), b"\x1bOS");
    assert_eq!(bytes_for(KeyName::F(5)), b"\x1b[15~");
    assert_eq!(bytes_for(KeyName::F(12)), b"\x1b[24~");
}

#[test]
fn key_name_parse_round_trip() {
    for &name in &[
        "enter",
        "tab",
        "escape",
        "backspace",
        "space",
        "up",
        "down",
        "left",
        "right",
        "home",
        "end",
        "pageup",
        "pagedown",
        "delete",
        "insert",
        "numpadenter",
    ] {
        assert!(KeyName::parse(name).is_some(), "expected {name} to parse");
    }
    for i in 1..=12 {
        assert!(
            KeyName::parse(&format!("f{i}")).is_some(),
            "expected f{i} to parse"
        );
    }
    assert!(KeyName::parse("frobnicate").is_none());
    assert!(KeyName::parse("f0").is_none());
    assert!(KeyName::parse("f13").is_none());
}
