//! Keyboard shortcuts: a key with modifiers (`Chord`), written and read as
//! text such as `Ctrl+Shift+S`, and the keyboard actions a workbench offers
//! beside its tools (`ActionDescriptor`).
//!
//! A workbench gives a tool a default key with `ToolDescriptor::shortcut`
//! and registers other actions with `WorkbenchContext::register_action`.
//! The application gathers these with its own commands into one keymap the
//! user can rebind; a triggered action reaches the workbench as
//! `WorkbenchInputEvent::Action`.

use std::fmt;
use std::str::FromStr;

use crate::KeyCode;

/// A key pressed with modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Chord {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub key: KeyCode,
}

impl Chord {
    /// A key with no modifiers.
    pub const fn key(key: KeyCode) -> Self {
        Self {
            ctrl: false,
            shift: false,
            alt: false,
            key,
        }
    }

    /// Whether a text field would take this chord as typing: a key with no
    /// Ctrl or Alt that is not a function key.
    pub fn types_text(&self) -> bool {
        !self.ctrl && !self.alt && !is_function_key(self.key)
    }

    /// Read `Ctrl+Shift+S`, `F`, `Ctrl+,` or `Alt+F4`. Modifiers and key
    /// names are case-insensitive; a key may be named or given as its
    /// character.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        // The key is what follows the last separator, so `Ctrl++` and
        // `Ctrl+,` keep their key.
        let (mods, key) = match text.rfind('+') {
            Some(at) if at + 1 < text.len() => (&text[..at], &text[at + 1..]),
            Some(at) if at > 0 => (&text[..at - 1], "+"),
            _ => ("", text),
        };
        let mut chord = Chord::key(key_from_name(key)?);
        for part in mods.split('+').map(str::trim).filter(|p| !p.is_empty()) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" | "cmd" | "command" => chord.ctrl = true,
                "shift" => chord.shift = true,
                "alt" | "option" => chord.alt = true,
                _ => return None,
            }
        }
        Some(chord)
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.ctrl {
            f.write_str("Ctrl+")?;
        }
        if self.alt {
            f.write_str("Alt+")?;
        }
        if self.shift {
            f.write_str("Shift+")?;
        }
        f.write_str(key_name(self.key))
    }
}

impl FromStr for Chord {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Chord::parse(text).ok_or_else(|| format!("`{text}` is not a key chord"))
    }
}

fn is_function_key(key: KeyCode) -> bool {
    use KeyCode::*;
    matches!(
        key,
        F1 | F2 | F3 | F4 | F5 | F6 | F7 | F8 | F9 | F10 | F11 | F12
    )
}

/// Every key a chord can name, with its written name.
const KEYS: &[(KeyCode, &str)] = {
    use KeyCode::*;
    &[
        (A, "A"),
        (B, "B"),
        (C, "C"),
        (D, "D"),
        (E, "E"),
        (F, "F"),
        (G, "G"),
        (H, "H"),
        (I, "I"),
        (J, "J"),
        (K, "K"),
        (L, "L"),
        (M, "M"),
        (N, "N"),
        (O, "O"),
        (P, "P"),
        (Q, "Q"),
        (R, "R"),
        (S, "S"),
        (T, "T"),
        (U, "U"),
        (V, "V"),
        (W, "W"),
        (X, "X"),
        (Y, "Y"),
        (Z, "Z"),
        (Key0, "0"),
        (Key1, "1"),
        (Key2, "2"),
        (Key3, "3"),
        (Key4, "4"),
        (Key5, "5"),
        (Key6, "6"),
        (Key7, "7"),
        (Key8, "8"),
        (Key9, "9"),
        (F1, "F1"),
        (F2, "F2"),
        (F3, "F3"),
        (F4, "F4"),
        (F5, "F5"),
        (F6, "F6"),
        (F7, "F7"),
        (F8, "F8"),
        (F9, "F9"),
        (F10, "F10"),
        (F11, "F11"),
        (F12, "F12"),
        (Escape, "Escape"),
        (Enter, "Enter"),
        (Space, "Space"),
        (Delete, "Delete"),
        (Backspace, "Backspace"),
        (Tab, "Tab"),
        (ArrowUp, "Up"),
        (ArrowDown, "Down"),
        (ArrowLeft, "Left"),
        (ArrowRight, "Right"),
        (Home, "Home"),
        (End, "End"),
        (PageUp, "PageUp"),
        (PageDown, "PageDown"),
        (Insert, "Insert"),
        (Period, "."),
        (Comma, ","),
        (Minus, "-"),
        (Equals, "="),
        (Slash, "/"),
    ]
};

/// Other names a key is read by.
const ALIASES: &[(&str, KeyCode)] = &[
    ("esc", KeyCode::Escape),
    ("return", KeyCode::Enter),
    ("del", KeyCode::Delete),
    ("period", KeyCode::Period),
    ("comma", KeyCode::Comma),
    ("minus", KeyCode::Minus),
    ("equals", KeyCode::Equals),
    ("slash", KeyCode::Slash),
    ("arrowup", KeyCode::ArrowUp),
    ("arrowdown", KeyCode::ArrowDown),
    ("arrowleft", KeyCode::ArrowLeft),
    ("arrowright", KeyCode::ArrowRight),
];

/// The written name of a key.
pub fn key_name(key: KeyCode) -> &'static str {
    KEYS.iter()
        .find(|(k, _)| *k == key)
        .map_or("?", |(_, name)| name)
}

fn key_from_name(name: &str) -> Option<KeyCode> {
    let name = name.trim();
    KEYS.iter()
        .find(|(_, n)| n.eq_ignore_ascii_case(name))
        .map(|(k, _)| *k)
        .or_else(|| {
            let lower = name.to_ascii_lowercase();
            ALIASES.iter().find(|(n, _)| *n == lower).map(|(_, k)| *k)
        })
}

/// A keyboard action a workbench offers that is not a tool: pressing its
/// shortcut while the workbench is active sends it
/// `WorkbenchInputEvent::Action { id }`.
#[derive(Debug, Clone)]
pub struct ActionDescriptor {
    /// Unique across the application, like a tool id; also the key the
    /// user's rebinding is saved under.
    pub id: String,
    pub label: String,
    /// Groups the action with the workbench's tools of the same category.
    pub category: Option<String>,
    /// The default keys; the user can change them.
    pub shortcuts: Vec<Chord>,
}

impl ActionDescriptor {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            category: None,
            shortcuts: Vec::new(),
        }
    }

    pub fn category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    /// Add a default key.
    ///
    /// # Panics
    ///
    /// When `chord` does not parse: a default key is written in the code,
    /// and a typo there is a bug to catch at registration.
    pub fn shortcut(mut self, chord: &str) -> Self {
        self.shortcuts.push(parse_default(chord));
        self
    }
}

/// A default key written in code, which must parse.
pub(crate) fn parse_default(chord: &str) -> Chord {
    Chord::parse(chord).unwrap_or_else(|| panic!("`{chord}` is not a key chord"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chords_read_and_write_the_same() {
        for text in [
            "Ctrl+Shift+S",
            "F",
            "Ctrl+,",
            "Alt+F4",
            "Ctrl+Tab",
            "Delete",
            "Ctrl+=",
            "Ctrl+Alt+Shift+Up",
        ] {
            let chord = Chord::parse(text).unwrap_or_else(|| panic!("{text}"));
            assert_eq!(chord.to_string(), text);
        }
    }

    #[test]
    fn names_are_case_insensitive_and_have_aliases() {
        let chord = Chord::parse("ctrl+shift+s").unwrap();
        assert!(chord.ctrl && chord.shift && !chord.alt);
        assert_eq!(chord.key, KeyCode::S);
        assert_eq!(Chord::parse("Esc").unwrap().key, KeyCode::Escape);
        assert_eq!(Chord::parse("Ctrl+Comma").unwrap().to_string(), "Ctrl+,");
        assert_eq!(Chord::parse("shift + a").unwrap().to_string(), "Shift+A");
    }

    #[test]
    fn nonsense_does_not_parse() {
        for text in ["", "Ctrl+", "Hyper+A", "Ctrl+Nope", "AB"] {
            assert!(Chord::parse(text).is_none(), "{text}");
        }
    }

    #[test]
    fn a_plain_letter_types_text_and_a_chord_does_not() {
        assert!(Chord::parse("L").unwrap().types_text());
        assert!(Chord::parse("Shift+L").unwrap().types_text());
        assert!(!Chord::parse("Ctrl+L").unwrap().types_text());
        assert!(!Chord::parse("F5").unwrap().types_text());
    }
}
