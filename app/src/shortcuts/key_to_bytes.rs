use crate::shortcuts::action::KeyName;

/// Default-mode (non-DECCKM) PTY byte sequence for a named key.
///
/// Returns the bytes that should be written to the active pane's PTY when the
/// custom-shortcut `press` action fires this key. Application Cursor Keys
/// mode (DECCKM) is not v1-aware — arrow keys always emit the `\x1b[?` form;
/// see PRODUCT.md follow-ups.
pub fn bytes_for(key: KeyName) -> &'static [u8] {
    match key {
        KeyName::Enter | KeyName::NumpadEnter => b"\r",
        KeyName::Tab => b"\t",
        KeyName::Escape => b"\x1b",
        KeyName::Backspace => b"\x7f",
        KeyName::Space => b" ",
        KeyName::Up => b"\x1b[A",
        KeyName::Down => b"\x1b[B",
        KeyName::Right => b"\x1b[C",
        KeyName::Left => b"\x1b[D",
        KeyName::Home => b"\x1b[H",
        KeyName::End => b"\x1b[F",
        KeyName::PageUp => b"\x1b[5~",
        KeyName::PageDown => b"\x1b[6~",
        KeyName::Insert => b"\x1b[2~",
        KeyName::Delete => b"\x1b[3~",
        KeyName::F(1) => b"\x1bOP",
        KeyName::F(2) => b"\x1bOQ",
        KeyName::F(3) => b"\x1bOR",
        KeyName::F(4) => b"\x1bOS",
        KeyName::F(5) => b"\x1b[15~",
        KeyName::F(6) => b"\x1b[17~",
        KeyName::F(7) => b"\x1b[18~",
        KeyName::F(8) => b"\x1b[19~",
        KeyName::F(9) => b"\x1b[20~",
        KeyName::F(10) => b"\x1b[21~",
        KeyName::F(11) => b"\x1b[23~",
        KeyName::F(12) => b"\x1b[24~",
        KeyName::F(_) => b"",
    }
}
