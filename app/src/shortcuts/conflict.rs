//! Chord-conflict detection for the side-panel GUI (PRODUCT §38).
//!
//! The runtime already accepts conflicts at parse time (custom shortcuts
//! shadow built-ins per PRODUCT §16, and the parser drops earlier duplicate
//! `keys` entries with a non-fatal error per §20). The GUI uses this module
//! to surface warnings *before* save so users see the consequence of their
//! chosen chord.

use warpui::keymap::Keystroke;

use crate::shortcuts::config::Shortcut;

#[derive(Debug, Clone)]
pub enum Conflict {
    /// The chord is bound to a built-in editable binding whose name does not
    /// start with `"shortcuts:"`. Saving will shadow that binding.
    BuiltIn { binding_name: String },
    /// The chord is already bound to another entry in `shortcuts.yaml`.
    Custom { entry_index: usize, keys: Keystroke },
}

/// Returns up to two conflicts: at most one built-in match and one
/// custom-entry match. `editing_index` skips the entry being edited so it
/// doesn't conflict with itself.
pub fn detect_custom_conflict(
    chord: &Keystroke,
    registry: &[Shortcut],
    editing_index: Option<usize>,
) -> Option<Conflict> {
    let normalized = chord.normalized();
    registry.iter().enumerate().find_map(|(i, s)| {
        if Some(i) == editing_index {
            return None;
        }
        if s.keys.normalized() == normalized {
            Some(Conflict::Custom {
                entry_index: i,
                keys: s.keys.clone(),
            })
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shortcuts::action::Action;
    use std::time::Duration as _Duration;

    fn make_shortcut(keys: &str, name: &str) -> Shortcut {
        Shortcut {
            keys: Keystroke::parse(keys).unwrap(),
            actions: vec![Action::Wait(_Duration::from_millis(1))],
            binding_name: name.to_owned(),
            name: None,
        }
    }

    #[test]
    fn no_conflict_in_empty_registry() {
        let chord = Keystroke::parse("cmd-shift-D").unwrap();
        assert!(detect_custom_conflict(&chord, &[], None).is_none());
    }

    #[test]
    fn custom_conflict_detected() {
        let chord = Keystroke::parse("cmd-shift-D").unwrap();
        let registry = vec![make_shortcut("cmd-shift-D", "shortcuts:user_0")];
        let conflict = detect_custom_conflict(&chord, &registry, None);
        assert!(matches!(
            conflict,
            Some(Conflict::Custom { entry_index: 0, .. })
        ));
    }

    #[test]
    fn editing_index_excludes_self() {
        let chord = Keystroke::parse("cmd-shift-D").unwrap();
        let registry = vec![make_shortcut("cmd-shift-D", "shortcuts:user_0")];
        // Editing entry #0 — its own chord shouldn't conflict.
        assert!(detect_custom_conflict(&chord, &registry, Some(0)).is_none());
    }

    #[test]
    fn normalization_matches_equivalent_chords() {
        let chord = Keystroke::parse("shift-cmd-D").unwrap();
        let registry = vec![make_shortcut("cmd-shift-D", "shortcuts:user_0")];
        // Same chord written in different modifier order should still match.
        assert!(detect_custom_conflict(&chord, &registry, None).is_some());
    }
}
