use std::time::Duration;

use crate::pane_group::Direction;

#[derive(Debug, Clone)]
pub enum Action {
    NewTab,
    NewPane(Direction),
    Type(String),
    Press(KeyName),
    Wait(Duration),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyName {
    Enter,
    Tab,
    Escape,
    Backspace,
    Space,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Delete,
    Insert,
    NumpadEnter,
    F(u8),
}

impl KeyName {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "enter" => Some(Self::Enter),
            "tab" => Some(Self::Tab),
            "escape" => Some(Self::Escape),
            "backspace" => Some(Self::Backspace),
            "space" => Some(Self::Space),
            "up" => Some(Self::Up),
            "down" => Some(Self::Down),
            "left" => Some(Self::Left),
            "right" => Some(Self::Right),
            "home" => Some(Self::Home),
            "end" => Some(Self::End),
            "pageup" => Some(Self::PageUp),
            "pagedown" => Some(Self::PageDown),
            "delete" => Some(Self::Delete),
            "insert" => Some(Self::Insert),
            "numpadenter" => Some(Self::NumpadEnter),
            other => {
                let digits = other.strip_prefix('f')?;
                let n: u8 = digits.parse().ok()?;
                (1..=12).contains(&n).then_some(Self::F(n))
            }
        }
    }
}
