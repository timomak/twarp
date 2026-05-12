use std::time::Duration;

use serde_yaml::Value;
use warpui::keymap::Keystroke;

use crate::pane_group::Direction;
use crate::shortcuts::action::{Action, KeyName};

#[derive(Debug, Clone)]
pub struct Shortcut {
    pub keys: Keystroke,
    pub actions: Vec<Action>,
    pub binding_name: String,
    /// Optional human-readable label for the side-panel list view
    /// (PRODUCT 04 §27). When `None`, the GUI falls back to an
    /// arrow-form action summary. Round-trips through serialize/parse.
    pub name: Option<String>,
}

#[derive(Debug, Default)]
pub struct ParseResult {
    pub shortcuts: Vec<Shortcut>,
    pub errors: Vec<String>,
}

pub fn parse_shortcuts_yaml(text: &str) -> ParseResult {
    if text.trim().is_empty() {
        return ParseResult::default();
    }

    let value: Value = match serde_yaml::from_str(text) {
        Ok(v) => v,
        Err(e) => {
            return ParseResult {
                shortcuts: Vec::new(),
                errors: vec![format!("shortcuts.yaml: failed to parse: {e}")],
            };
        }
    };

    let mapping = match &value {
        Value::Null => return ParseResult::default(),
        Value::Mapping(m) => m,
        _ => {
            return ParseResult {
                shortcuts: Vec::new(),
                errors: vec![
                    "shortcuts.yaml: expected top-level 'shortcuts:' key with a list value"
                        .to_owned(),
                ],
            };
        }
    };

    let mut errors = Vec::new();

    // The only recognized top-level key is `shortcuts`. Anything else is rejected.
    for (k, _) in mapping {
        let key = k.as_str();
        if key != Some("shortcuts") {
            return ParseResult {
                shortcuts: Vec::new(),
                errors: vec![
                    "shortcuts.yaml: expected top-level 'shortcuts:' key with a list value"
                        .to_owned(),
                ],
            };
        }
    }

    let shortcuts_value = match mapping.get(&Value::String("shortcuts".to_owned())) {
        Some(v) => v,
        None => {
            return ParseResult {
                shortcuts: Vec::new(),
                errors: vec![
                    "shortcuts.yaml: expected top-level 'shortcuts:' key with a list value"
                        .to_owned(),
                ],
            };
        }
    };

    let entries = match shortcuts_value {
        Value::Sequence(seq) => seq,
        Value::Null => return ParseResult::default(),
        other => {
            let kind = yaml_type_name(other);
            return ParseResult {
                shortcuts: Vec::new(),
                errors: vec![format!(
                    "shortcuts.yaml: 'shortcuts:' must be a list, got {kind}"
                )],
            };
        }
    };

    let mut shortcuts: Vec<Shortcut> = Vec::with_capacity(entries.len());

    for (i, entry) in entries.iter().enumerate() {
        let entry_idx = i + 1;
        match parse_entry(entry, entry_idx) {
            Ok(sc) => {
                if let Some(dup_pos) = shortcuts
                    .iter()
                    .position(|s| s.keys.normalized() == sc.keys.normalized())
                {
                    errors.push(format!(
                        "shortcuts.yaml: entry #{entry_idx} ('{keys}') is a duplicate of an earlier entry; using the last definition",
                        keys = sc.keys.normalized()
                    ));
                    shortcuts.remove(dup_pos);
                }
                shortcuts.push(sc);
            }
            Err(msg) => errors.push(format!("shortcuts.yaml: {msg}")),
        }
    }

    for (i, sc) in shortcuts.iter_mut().enumerate() {
        sc.binding_name = format!("shortcuts:user_{i}");
    }

    ParseResult { shortcuts, errors }
}

fn parse_entry(entry: &Value, entry_idx: usize) -> Result<Shortcut, String> {
    let mapping = match entry {
        Value::Mapping(m) => m,
        _ => {
            return Err(format!("entry #{entry_idx}: missing required field 'keys'"));
        }
    };

    let mut keys_raw: Option<String> = None;
    let mut actions_value: Option<&Value> = None;
    let mut name: Option<String> = None;

    for (k, v) in mapping {
        let key = match k.as_str() {
            Some(s) => s,
            None => {
                return Err(format!("entry #{entry_idx}: missing required field 'keys'"));
            }
        };
        match key {
            "keys" => {
                keys_raw = v.as_str().map(|s| s.to_owned());
            }
            "actions" => {
                actions_value = Some(v);
            }
            "name" => {
                // PRODUCT §3: optional `name` field for the GUI list
                // view (4c). Null or empty string is treated as
                // unset; non-string values are silently ignored
                // rather than erroring, so existing files without a
                // name keep working unchanged.
                name = v.as_str().and_then(|s| {
                    let trimmed = s.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.to_owned())
                    }
                });
            }
            other => {
                let keys_display = keys_raw.clone().unwrap_or_default();
                return Err(format!(
                    "entry #{entry_idx} ('{keys_display}'): unknown field '{other}'; expected 'keys', 'actions', and optionally 'name'"
                ));
            }
        }
    }

    let keys_str =
        keys_raw.ok_or_else(|| format!("entry #{entry_idx}: missing required field 'keys'"))?;

    let normalized_chord = normalize_chord_casing(&keys_str);
    let keys = Keystroke::parse(&normalized_chord).map_err(|_| {
        format!(
            "entry #{entry_idx}: invalid key chord '{keys_str}'; expected modifiers (cmdorctrl/cmd/ctrl/alt/shift/meta) joined by '-' with a key, e.g. 'cmdorctrl-shift-D'"
        )
    })?;
    let keys_display = keys.normalized();

    let actions_value = actions_value.ok_or_else(|| {
        format!(
            "entry #{entry_idx} ('{keys_display}'): missing required field 'actions' (must have at least one action)"
        )
    })?;

    let actions_seq = match actions_value {
        Value::Sequence(seq) if !seq.is_empty() => seq,
        _ => {
            return Err(format!(
                "entry #{entry_idx} ('{keys_display}'): missing required field 'actions' (must have at least one action)"
            ));
        }
    };

    let mut actions: Vec<Action> = Vec::with_capacity(actions_seq.len());
    for (j, item) in actions_seq.iter().enumerate() {
        let action_idx = j + 1;
        let act = parse_action(item, entry_idx, &keys_display, action_idx)?;
        actions.push(act);
    }

    Ok(Shortcut {
        keys,
        actions,
        binding_name: String::new(),
        name,
    })
}

fn parse_action(
    item: &Value,
    entry_idx: usize,
    keys: &str,
    action_idx: usize,
) -> Result<Action, String> {
    let prefix = format!("entry #{entry_idx} ('{keys}'), action #{action_idx}");

    if let Some(name) = item.as_str() {
        return match name {
            "new_tab" => Ok(Action::NewTab),
            "new_pane" => Err(format!(
                "{prefix}: 'new_pane' requires a direction; expected 'right', 'down', 'left', or 'up'"
            )),
            other if is_known_action_with_value(other) => Err(format!(
                "{prefix}: expected a bare action name or a single-key map"
            )),
            other => Err(format!(
                "{prefix}: unknown action '{other}'; expected one of new_tab, new_pane, type, press, wait"
            )),
        };
    }

    let mapping = match item {
        Value::Mapping(m) => m,
        _ => {
            return Err(format!(
                "{prefix}: expected a bare action name or a single-key map"
            ));
        }
    };

    if mapping.len() != 1 {
        return Err(format!(
            "{prefix}: expected a bare action name or a single-key map"
        ));
    }

    let (k, v) = mapping.iter().next().unwrap();
    let name = match k.as_str() {
        Some(s) => s,
        None => {
            return Err(format!(
                "{prefix}: expected a bare action name or a single-key map"
            ));
        }
    };

    match name {
        "new_tab" => Err(format!(
            "{prefix}: expected a bare action name or a single-key map"
        )),
        "new_pane" => parse_new_pane(v, &prefix),
        "type" => parse_type(v, &prefix),
        "press" => parse_press(v, &prefix),
        "wait" => parse_wait(v, &prefix),
        other => Err(format!(
            "{prefix}: unknown action '{other}'; expected one of new_tab, new_pane, type, press, wait"
        )),
    }
}

fn parse_new_pane(value: &Value, prefix: &str) -> Result<Action, String> {
    let raw = match value {
        Value::String(s) => s.as_str(),
        Value::Null => {
            return Err(format!(
                "{prefix}: 'new_pane' requires a direction; expected 'right', 'down', 'left', or 'up'"
            ));
        }
        _ => {
            let raw = serde_yaml::to_string(value).unwrap_or_default();
            let raw = raw.trim().trim_end_matches('\n').to_owned();
            return Err(format!(
                "{prefix}: invalid 'new_pane' direction '{raw}'; expected 'right', 'down', 'left', or 'up'"
            ));
        }
    };
    match raw {
        "right" => Ok(Action::NewPane(Direction::Right)),
        "down" => Ok(Action::NewPane(Direction::Down)),
        "left" => Ok(Action::NewPane(Direction::Left)),
        "up" => Ok(Action::NewPane(Direction::Up)),
        other => Err(format!(
            "{prefix}: invalid 'new_pane' direction '{other}'; expected 'right', 'down', 'left', or 'up'"
        )),
    }
}

fn parse_type(value: &Value, prefix: &str) -> Result<Action, String> {
    let text = match value {
        Value::String(s) => s.clone(),
        _ => return Err(format!("{prefix}: 'type' expects a string value")),
    };
    if text.contains('\n') || text.contains('\r') {
        return Err(format!(
            "{prefix}: 'type' value contains a newline; use 'press: enter' to submit input"
        ));
    }
    Ok(Action::Type(text))
}

fn parse_press(value: &Value, prefix: &str) -> Result<Action, String> {
    let raw = match value {
        Value::String(s) => s.as_str(),
        _ => {
            return Err(format!(
                "{prefix}: unknown key '' in 'press'; expected one of enter, tab, escape, backspace, space, up, down, left, right, home, end, pageup, pagedown, delete, insert, numpadenter, f1-f12"
            ));
        }
    };
    if raw.contains('-') || raw.contains('+') {
        return Err(format!(
            "{prefix}: 'press' does not support modifiers in v1 (got '{raw}')"
        ));
    }
    KeyName::parse(raw).map(Action::Press).ok_or_else(|| {
        format!(
            "{prefix}: unknown key '{raw}' in 'press'; expected one of enter, tab, escape, backspace, space, up, down, left, right, home, end, pageup, pagedown, delete, insert, numpadenter, f1-f12"
        )
    })
}

fn parse_wait(value: &Value, prefix: &str) -> Result<Action, String> {
    let raw = match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => {
            return Err(format!(
                "{prefix}: invalid 'wait' value ''; expected a duration like '500ms', '2s', '1m' (1ms–60s)"
            ));
        }
    };
    let dur = parse_duration(&raw).ok_or_else(|| {
        format!(
            "{prefix}: invalid 'wait' value '{raw}'; expected a duration like '500ms', '2s', '1m' (1ms–60s)"
        )
    })?;
    if dur < Duration::from_millis(1) || dur > Duration::from_secs(60) {
        return Err(format!(
            "{prefix}: invalid 'wait' value '{raw}'; expected a duration like '500ms', '2s', '1m' (1ms–60s)"
        ));
    }
    Ok(Action::Wait(dur))
}

fn parse_duration(raw: &str) -> Option<Duration> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (digits, unit) = if let Some(rest) = trimmed.strip_suffix("ms") {
        (rest, "ms")
    } else if let Some(rest) = trimmed.strip_suffix('s') {
        (rest, "s")
    } else if let Some(rest) = trimmed.strip_suffix('m') {
        (rest, "m")
    } else {
        return None;
    };
    let n: u64 = digits.parse().ok()?;
    let dur = match unit {
        "ms" => Duration::from_millis(n),
        "s" => Duration::from_secs(n),
        "m" => Duration::from_secs(n.checked_mul(60)?),
        _ => return None,
    };
    Some(dur)
}

/// Normalize `shift-<lowercase letter>` chords to uppercase so they pass
/// `Keystroke::parse` (which debug-panics on `shift-<lowercase>`).
fn normalize_chord_casing(chord: &str) -> String {
    let parts: Vec<&str> = chord.split('-').collect();
    let has_shift = parts.iter().any(|p| p.eq_ignore_ascii_case("shift"));
    if !has_shift {
        return chord.to_owned();
    }
    let mut out: Vec<String> = Vec::with_capacity(parts.len());
    let last_idx = parts.len() - 1;
    for (i, part) in parts.iter().enumerate() {
        if i == last_idx && part.len() == 1 {
            let ch = part.chars().next().unwrap();
            if ch.is_ascii_lowercase() {
                out.push(ch.to_ascii_uppercase().to_string());
                continue;
            }
        }
        out.push((*part).to_owned());
    }
    out.join("-")
}

/// Serialize a list of shortcuts back to YAML, in the canonical shape the
/// parser accepts (PRODUCT §37). Comments and hand-formatting are dropped;
/// within each entry, `keys` precedes `actions`; within each action item,
/// single-key maps are compact (`type: claude`, not multi-line). The
/// parse(serialize(x)) round-trip is asserted by the
/// `serialize_round_trips_through_parser` test.
pub fn serialize_shortcuts(shortcuts: &[Shortcut]) -> String {
    if shortcuts.is_empty() {
        return "shortcuts: []\n".to_owned();
    }
    let mut out = String::from("shortcuts:\n");
    for sc in shortcuts {
        out.push_str(&format!("  - keys: {}\n", sc.keys.normalized()));
        if let Some(name) = &sc.name {
            out.push_str(&format!("    name: {}\n", yaml_escape_string(name)));
        }
        out.push_str("    actions:\n");
        for action in &sc.actions {
            match action {
                Action::NewTab => out.push_str("      - new_tab\n"),
                Action::NewPane(direction) => {
                    let name = direction_name(*direction);
                    out.push_str(&format!("      - new_pane: {name}\n"));
                }
                Action::Type(text) => {
                    out.push_str(&format!("      - type: {}\n", yaml_escape_string(text)));
                }
                Action::Press(key) => {
                    out.push_str(&format!("      - press: {}\n", key_name_to_yaml(*key)));
                }
                Action::Wait(duration) => {
                    out.push_str(&format!("      - wait: {}\n", duration_to_yaml(*duration)));
                }
            }
        }
    }
    out
}

fn direction_name(d: Direction) -> &'static str {
    match d {
        Direction::Right => "right",
        Direction::Down => "down",
        Direction::Left => "left",
        Direction::Up => "up",
    }
}

fn key_name_to_yaml(key: KeyName) -> String {
    match key {
        KeyName::Enter => "enter".to_owned(),
        KeyName::Tab => "tab".to_owned(),
        KeyName::Escape => "escape".to_owned(),
        KeyName::Backspace => "backspace".to_owned(),
        KeyName::Space => "space".to_owned(),
        KeyName::Up => "up".to_owned(),
        KeyName::Down => "down".to_owned(),
        KeyName::Left => "left".to_owned(),
        KeyName::Right => "right".to_owned(),
        KeyName::Home => "home".to_owned(),
        KeyName::End => "end".to_owned(),
        KeyName::PageUp => "pageup".to_owned(),
        KeyName::PageDown => "pagedown".to_owned(),
        KeyName::Delete => "delete".to_owned(),
        KeyName::Insert => "insert".to_owned(),
        KeyName::NumpadEnter => "numpadenter".to_owned(),
        KeyName::F(n) => format!("f{n}"),
    }
}

fn duration_to_yaml(d: Duration) -> String {
    let total_ms = d.as_millis();
    if total_ms.is_multiple_of(60_000) {
        format!("{}m", total_ms / 60_000)
    } else if total_ms.is_multiple_of(1000) {
        format!("{}s", total_ms / 1000)
    } else {
        format!("{total_ms}ms")
    }
}

fn yaml_escape_string(s: &str) -> String {
    // PRODUCT §9 already rejects newlines in `type`. We still wrap in double
    // quotes for clarity and to handle special characters; serde_yaml-style
    // escapes are sufficient since type values are user text without
    // line breaks.
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

fn yaml_type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Sequence(_) => "list",
        Value::Mapping(_) => "map",
    }
}

fn is_known_action_with_value(name: &str) -> bool {
    matches!(name, "type" | "press" | "wait" | "new_pane")
}
