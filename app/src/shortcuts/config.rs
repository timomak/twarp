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
            other => {
                let keys_display = keys_raw.clone().unwrap_or_default();
                return Err(format!(
                    "entry #{entry_idx} ('{keys_display}'): unknown field '{other}'; expected 'keys' and 'actions'"
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
                "{prefix}: 'new_pane' requires a direction; expected 'right' or 'down'"
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
                "{prefix}: 'new_pane' requires a direction; expected 'right' or 'down'"
            ));
        }
        _ => {
            let raw = serde_yaml::to_string(value).unwrap_or_default();
            let raw = raw.trim().trim_end_matches('\n').to_owned();
            return Err(format!(
                "{prefix}: invalid 'new_pane' direction '{raw}'; expected 'right' or 'down'"
            ));
        }
    };
    match raw {
        "right" => Ok(Action::NewPane(Direction::Right)),
        "down" => Ok(Action::NewPane(Direction::Down)),
        other => Err(format!(
            "{prefix}: invalid 'new_pane' direction '{other}'; expected 'right' or 'down'"
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
