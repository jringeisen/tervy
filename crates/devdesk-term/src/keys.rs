//! GPUI-free terminal input encoding. Protocol references: xterm ctlseqs and
//! kitty's keyboard protocol (https://sw.kovidgoyal.net/kitty/keyboard-protocol/).
//! Original implementation; no Zed terminal/editor code is used.

use crate::TermMode;

#[derive(Clone, Copy, Debug, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub alt: bool,
    pub control: bool,
    pub super_key: bool,
}

impl Modifiers {
    pub fn parameter(self) -> u8 {
        1 + u8::from(self.shift)
            + 2 * u8::from(self.alt)
            + 4 * u8::from(self.control)
            + 8 * u8::from(self.super_key)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Character(char),
    Enter,
    Tab,
    Backspace,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    Insert,
    Delete,
    PageUp,
    PageDown,
    Function(u8),
    Keypad(u8),
    KeypadEnter,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeyEvent {
    #[default]
    Press,
    Repeat,
    Release,
}

pub struct KeyInput {
    pub key: Key,
    pub modifiers: Modifiers,
    pub event: KeyEvent,
    pub text: Option<String>,
    pub shifted: Option<char>,
    pub base_layout: Option<char>,
}

pub fn encode(input: &KeyInput, mode: TermMode, option_as_meta: bool) -> Vec<u8> {
    let mut modifiers = input.modifiers;
    // Option can compose accents through the native input handler. Only treat
    // it as Meta when explicitly requested by the user.
    modifiers.alt &= option_as_meta;
    if input.event == KeyEvent::Release && !mode.contains(TermMode::REPORT_EVENT_TYPES) {
        return Vec::new();
    }
    let kitty = mode.intersects(TermMode::KITTY_KEYBOARD_PROTOCOL);
    let force = mode.contains(TermMode::REPORT_ALL_KEYS_AS_ESC);
    let modified = modifiers.parameter() != 1;
    let ambiguous = matches!(input.key, Key::Escape)
        || (modified && matches!(input.key, Key::Enter | Key::Tab | Key::Backspace))
        || matches!(input.key, Key::Keypad(_) | Key::KeypadEnter)
        || modifiers.control
        || modifiers.alt
        || modifiers.super_key;
    if input.event == KeyEvent::Release
        && !force
        && (matches!(input.key, Key::Enter | Key::Tab | Key::Backspace)
            || (matches!(input.key, Key::Character(_)) && !ambiguous))
    {
        return Vec::new();
    }
    let encoded = kitty && (force || ambiguous || input.event == KeyEvent::Release);
    if encoded && let Some(code) = unicode_key(input.key) {
        let mut key = code.to_string();
        if mode.contains(TermMode::REPORT_ALTERNATE_KEYS) {
            if let Some(shifted) = input.shifted {
                key.push_str(&format!(":{}", shifted as u32));
            }
            if let Some(base) = input.base_layout {
                if input.shifted.is_none() {
                    key.push(':');
                }
                key.push_str(&format!(":{}", base as u32));
            }
        }
        let event = if mode.contains(TermMode::REPORT_EVENT_TYPES) {
            match input.event {
                KeyEvent::Press => ":1",
                KeyEvent::Repeat => ":2",
                KeyEvent::Release => ":3",
            }
        } else {
            ""
        };
        let text = if mode.contains(TermMode::REPORT_ASSOCIATED_TEXT)
            && input.event != KeyEvent::Release
        {
            input
                .text
                .as_deref()
                .filter(|text| !text.is_empty())
                .map(|text| {
                    format!(
                        ";{}",
                        text.chars()
                            .map(|c| (c as u32).to_string())
                            .collect::<Vec<_>>()
                            .join(":")
                    )
                })
                .unwrap_or_default()
        } else {
            String::new()
        };
        return format!("\x1b[{key};{}{event}{text}u", modifiers.parameter()).into_bytes();
    }
    let suffix = match input.key {
        Key::Up => Some('A'),
        Key::Down => Some('B'),
        Key::Right => Some('C'),
        Key::Left => Some('D'),
        Key::Home => Some('H'),
        Key::End => Some('F'),
        Key::Function(1) => Some('P'),
        Key::Function(2) => Some('Q'),
        Key::Function(3) if !kitty => Some('R'),
        Key::Function(4) => Some('S'),
        _ => None,
    };
    if let Some(suffix) = suffix {
        let event = if mode.contains(TermMode::REPORT_EVENT_TYPES) {
            match input.event {
                KeyEvent::Press => "",
                KeyEvent::Repeat => ":2",
                KeyEvent::Release => ":3",
            }
        } else {
            ""
        };
        if modified || !event.is_empty() || force {
            return format!("\x1b[1;{}{event}{suffix}", modifiers.parameter()).into_bytes();
        }
        let application = !kitty
            && (mode.contains(TermMode::APP_CURSOR) || matches!(input.key, Key::Function(_)));
        return format!("\x1b{}{suffix}", if application { 'O' } else { '[' }).into_bytes();
    }
    let tilde = match input.key {
        Key::Insert => Some(2),
        Key::Delete => Some(3),
        Key::PageUp => Some(5),
        Key::PageDown => Some(6),
        Key::Function(3) if kitty => Some(13),
        Key::Function(n @ 5..=12) => Some([15, 17, 18, 19, 20, 21, 23, 24][(n - 5) as usize]),
        _ => None,
    };
    if let Some(code) = tilde {
        let event = if mode.contains(TermMode::REPORT_EVENT_TYPES) {
            match input.event {
                KeyEvent::Press => "",
                KeyEvent::Repeat => ":2",
                KeyEvent::Release => ":3",
            }
        } else {
            ""
        };
        return if modified || !event.is_empty() || force {
            format!("\x1b[{code};{}{event}~", modifiers.parameter()).into_bytes()
        } else {
            format!("\x1b[{code}~").into_bytes()
        };
    }
    let mut bytes = match input.key {
        Key::Enter if modifiers.shift => b"\x1b[13;2u".to_vec(),
        Key::KeypadEnter if mode.contains(TermMode::APP_KEYPAD) => b"\x1bOM".to_vec(),
        Key::Enter | Key::KeypadEnter => vec![b'\r'],
        Key::Backspace => vec![if modifiers.control { 8 } else { 127 }],
        Key::Escape => vec![27],
        Key::Tab if modifiers.shift => b"\x1b[Z".to_vec(),
        Key::Tab => vec![9],
        Key::Keypad(n @ 0..=9) if mode.contains(TermMode::APP_KEYPAD) => vec![27, b'O', b'p' + n],
        Key::Keypad(n @ 0..=9) => vec![b'0' + n],
        Key::Character(c) if modifiers.control => match c.to_ascii_lowercase() {
            ' ' | '@' | '2' => vec![0],
            'a'..='z' => vec![c.to_ascii_lowercase() as u8 - b'a' + 1],
            '[' | '3' => vec![27],
            '\\' | '4' => vec![28],
            ']' | '5' => vec![29],
            '^' | '6' => vec![30],
            '_' | '7' | '/' => vec![31],
            '?' | '8' => vec![127],
            _ => input
                .text
                .clone()
                .unwrap_or_else(|| c.to_string())
                .into_bytes(),
        },
        Key::Character(c) => input
            .text
            .clone()
            .unwrap_or_else(|| c.to_string())
            .into_bytes(),
        _ => Vec::new(),
    };
    if modifiers.alt && !bytes.is_empty() {
        bytes.insert(0, 27);
    }
    bytes
}

fn unicode_key(key: Key) -> Option<u32> {
    Some(match key {
        Key::Character(c) => c as u32,
        Key::Enter => 13,
        Key::Tab => 9,
        Key::Backspace => 127,
        Key::Escape => 27,
        Key::Keypad(n @ 0..=9) => 57399 + n as u32,
        Key::KeypadEnter => 57414,
        Key::Function(n @ 13..=35) => 57364 + n as u32 - 1,
        _ => return None,
    })
}

pub fn paste(text: &str, mode: TermMode) -> Vec<u8> {
    // Never permit a clipboard escape byte to terminate bracketed paste or
    // inject commands. In legacy mode normalize line endings to Return.
    let text = text.replace('\x1b', "");
    if mode.contains(TermMode::BRACKETED_PASTE) {
        format!("\x1b[200~{text}\x1b[201~").into_bytes()
    } else {
        text.replace("\r\n", "\n").replace('\n', "\r").into_bytes()
    }
}

pub fn focus(focused: bool, mode: TermMode) -> Vec<u8> {
    if mode.contains(TermMode::FOCUS_IN_OUT) {
        if focused {
            b"\x1b[I".to_vec()
        } else {
            b"\x1b[O".to_vec()
        }
    } else {
        Vec::new()
    }
}

#[derive(Clone, Copy)]
pub enum MouseAction {
    Press(u8),
    Release(u8),
    Motion(Option<u8>),
    WheelUp,
    WheelDown,
}

/// Zero-based cell coordinates. SGR is preferred; legacy and UTF-8 mouse
/// reporting remain necessary for TUIs that enable 1000 without 1006.
pub fn mouse(
    action: MouseAction,
    column: usize,
    row: usize,
    modifiers: Modifiers,
    mode: TermMode,
) -> Vec<u8> {
    if !mode.intersects(TermMode::MOUSE_MODE) {
        return Vec::new();
    }
    let (mut code, release) = match action {
        MouseAction::Press(button) => (button, false),
        MouseAction::Release(button) => (button, true),
        MouseAction::WheelUp => (64, false),
        MouseAction::WheelDown => (65, false),
        MouseAction::Motion(button) => {
            if !mode.contains(TermMode::MOUSE_MOTION)
                && (!mode.contains(TermMode::MOUSE_DRAG) || button.is_none())
            {
                return Vec::new();
            }
            (32 + button.unwrap_or(3), false)
        }
    };
    code += 4 * u8::from(modifiers.shift)
        + 8 * u8::from(modifiers.alt)
        + 16 * u8::from(modifiers.control);
    if mode.contains(TermMode::SGR_MOUSE) {
        return format!(
            "\x1b[<{code};{};{}{}",
            column + 1,
            row + 1,
            if release { 'm' } else { 'M' }
        )
        .into_bytes();
    }
    if release {
        code = 3 + (code & 28);
    }
    let mut bytes = vec![27, b'[', b'M', code + 32];
    for coordinate in [column + 33, row + 33] {
        if mode.contains(TermMode::UTF8_MOUSE) {
            let Some(c) = char::from_u32(coordinate as u32) else {
                return Vec::new();
            };
            let mut buffer = [0; 4];
            bytes.extend_from_slice(c.encode_utf8(&mut buffer).as_bytes());
        } else {
            if coordinate > 255 {
                return Vec::new();
            }
            bytes.push(coordinate as u8);
        }
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input(key: Key, modifiers: Modifiers) -> KeyInput {
        KeyInput {
            key,
            modifiers,
            event: KeyEvent::Press,
            text: None,
            shifted: None,
            base_layout: None,
        }
    }

    #[test]
    fn legacy_and_kitty_sequences() {
        let shift = Modifiers {
            shift: true,
            ..Modifiers::default()
        };
        let ctrl = Modifiers {
            control: true,
            ..Modifiers::default()
        };
        let table = [
            (Key::Up, Modifiers::default(), TermMode::NONE, "\x1b[A"),
            (
                Key::Up,
                Modifiers::default(),
                TermMode::APP_CURSOR,
                "\x1bOA",
            ),
            (Key::Up, shift, TermMode::NONE, "\x1b[1;2A"),
            (Key::Character('c'), ctrl, TermMode::NONE, "\x03"),
            (
                Key::Enter,
                shift,
                TermMode::DISAMBIGUATE_ESC_CODES,
                "\x1b[13;2u",
            ),
            (
                Key::Escape,
                Modifiers::default(),
                TermMode::DISAMBIGUATE_ESC_CODES,
                "\x1b[27;1u",
            ),
            (Key::Delete, ctrl, TermMode::NONE, "\x1b[3;5~"),
            (
                Key::Function(5),
                Modifiers::default(),
                TermMode::NONE,
                "\x1b[15~",
            ),
        ];
        for (key, modifiers, mode, expected) in table {
            assert_eq!(
                encode(&input(key, modifiers), mode, false),
                expected.as_bytes()
            );
        }
    }

    #[test]
    fn kitty_release_and_associated_unicode_text() {
        let mut input = input(
            Key::Character('a'),
            Modifiers {
                shift: true,
                ..Modifiers::default()
            },
        );
        input.text = Some("Ä".into());
        input.shifted = Some('A');
        let mode = TermMode::REPORT_ALL_KEYS_AS_ESC
            | TermMode::REPORT_EVENT_TYPES
            | TermMode::REPORT_ASSOCIATED_TEXT
            | TermMode::REPORT_ALTERNATE_KEYS;
        assert_eq!(encode(&input, mode, false), b"\x1b[97:65;2:1;196u");
        input.event = KeyEvent::Release;
        assert_eq!(encode(&input, mode, false), b"\x1b[97:65;2:3u");
        assert!(encode(&input, TermMode::NONE, false).is_empty());
    }

    #[test]
    fn paste_focus_and_mouse_protocols() {
        assert_eq!(
            paste("a\nb\x1b", TermMode::BRACKETED_PASTE),
            b"\x1b[200~a\nb\x1b[201~"
        );
        assert_eq!(paste("a\r\nb", TermMode::NONE), b"a\rb");
        assert_eq!(focus(true, TermMode::FOCUS_IN_OUT), b"\x1b[I");
        let mode = TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE;
        assert_eq!(
            mouse(MouseAction::Press(0), 2, 3, Modifiers::default(), mode),
            b"\x1b[<0;3;4M"
        );
        assert_eq!(
            mouse(MouseAction::Release(0), 2, 3, Modifiers::default(), mode),
            b"\x1b[<0;3;4m"
        );
        assert!(mouse(MouseAction::Motion(None), 2, 3, Modifiers::default(), mode).is_empty());
    }
}

#[cfg(test)]
mod protocol_edges {
    use super::*;
    fn input(key: Key, modifiers: Modifiers, event: KeyEvent) -> KeyInput {
        KeyInput {
            key,
            modifiers,
            event,
            text: None,
            shifted: None,
            base_layout: None,
        }
    }
    #[test]
    fn keypad_and_f3_do_not_collide_with_cursor_queries() {
        let none = Modifiers::default();
        assert_eq!(
            encode(
                &input(Key::Function(3), none, KeyEvent::Press),
                TermMode::DISAMBIGUATE_ESC_CODES,
                false
            ),
            b"\x1b[13~"
        );
        assert_eq!(
            encode(
                &input(Key::KeypadEnter, none, KeyEvent::Press),
                TermMode::APP_KEYPAD,
                false
            ),
            b"\x1bOM"
        );
        assert_eq!(
            encode(
                &input(Key::Keypad(2), none, KeyEvent::Press),
                TermMode::APP_KEYPAD,
                false
            ),
            b"\x1bOr"
        );
        assert_eq!(
            encode(
                &input(Key::Keypad(2), none, KeyEvent::Press),
                TermMode::DISAMBIGUATE_ESC_CODES,
                false
            ),
            b"\x1b[57401;1u"
        );
    }
    #[test]
    fn modified_character_releases_are_reported_but_plain_text_is_not() {
        let mode = TermMode::DISAMBIGUATE_ESC_CODES | TermMode::REPORT_EVENT_TYPES;
        let ctrl = Modifiers {
            control: true,
            ..Modifiers::default()
        };
        assert_eq!(
            encode(
                &input(Key::Character('c'), ctrl, KeyEvent::Release),
                mode,
                false
            ),
            b"\x1b[99;5:3u"
        );
        assert!(
            encode(
                &input(Key::Character('c'), Modifiers::default(), KeyEvent::Release),
                mode,
                false
            )
            .is_empty()
        );
        assert_eq!(
            encode(
                &input(Key::Character('9'), ctrl, KeyEvent::Press),
                TermMode::NONE,
                false
            ),
            b"9"
        );
    }
    #[test]
    fn option_is_meta_only_when_requested_and_mouse_motion_obeys_drag_mode() {
        let alt = Modifiers {
            alt: true,
            ..Modifiers::default()
        };
        let key = input(Key::Character('x'), alt, KeyEvent::Press);
        assert_eq!(encode(&key, TermMode::NONE, false), b"x");
        assert_eq!(encode(&key, TermMode::NONE, true), b"\x1bx");
        let mode = TermMode::MOUSE_DRAG | TermMode::SGR_MOUSE;
        assert!(mouse(MouseAction::Motion(None), 0, 0, Modifiers::default(), mode).is_empty());
        assert_eq!(
            mouse(
                MouseAction::Motion(Some(0)),
                0,
                0,
                Modifiers::default(),
                mode
            ),
            b"\x1b[<32;1;1M"
        );
        assert_eq!(
            mouse(MouseAction::WheelDown, 0, 0, Modifiers::default(), mode),
            b"\x1b[<65;1;1M"
        );
    }
}
