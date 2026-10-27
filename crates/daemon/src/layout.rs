//! Character → key-press translation for the user's keyboard layout.
//!
//! The virtual keyboard can only send key positions; the desktop's layout decides which
//! character each position types. To type a given character we therefore ask the platform
//! (libxkbcommon on Linux, the active keyboard layout on Windows) which key + modifiers
//! produce it. If that fails we fall back to US.

use std::collections::HashMap;

use x360truewireless_common::keycode::KeyCode;

/// Key presses that type one character.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Combo {
    /// Chords tapped (pressed and released) first, e.g. a dead key.
    pub taps: Vec<Vec<KeyCode>>,
    /// Chord held down until the chatpad key is released.
    pub hold: Vec<KeyCode>,
}

impl Combo {
    pub fn key(k: KeyCode) -> Self {
        Combo {
            taps: Vec::new(),
            hold: vec![k],
        }
    }

    /// `c` typed as text rather than with keys, for characters the layout has no key for
    /// (Windows only, where `SendInput` can type any character). Stored as a key code from
    /// [`TEXT_BASE`] up, past every real key.
    #[cfg(windows)]
    pub fn text(c: char) -> Option<Self> {
        let code = u16::try_from(TEXT_BASE as u32 + c as u32).ok()?;
        Some(Combo::key(KeyCode(code)))
    }
}

/// Where [`Combo::text`]'s key codes start: past `KEY_MAX` (0x2ff).
#[cfg(windows)]
pub const TEXT_BASE: u16 = 0x300;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LayoutName {
    pub layout: String,
    pub variant: String,
}

impl std::fmt::Display for LayoutName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.variant.is_empty() {
            write!(f, "{}", self.layout)
        } else {
            write!(f, "{}({})", self.layout, self.variant)
        }
    }
}

pub struct Layout {
    pub name: LayoutName,
    pub(crate) chars: HashMap<char, Combo>,
}

/// Modifier chords tried for each key, in order of preference.
pub const MODIFIER_CHORDS: &[&[KeyCode]] = &[
    &[],
    &[KeyCode::KEY_LEFTSHIFT],
    &[KeyCode::KEY_RIGHTALT],
    &[KeyCode::KEY_LEFTSHIFT, KeyCode::KEY_RIGHTALT],
];

/// Keys that only exist on some keyboards (ISO, Japanese); used only if nothing else works.
const EXTRA_KEYS: &[KeyCode] = &[KeyCode::KEY_102ND, KeyCode::KEY_RO, KeyCode::KEY_YEN];

/// The main typing block. Keypad keys are excluded since they depend on NumLock.
pub fn typing_keys() -> impl Iterator<Item = KeyCode> {
    let rows = [
        (KeyCode::KEY_1, KeyCode::KEY_EQUAL),
        (KeyCode::KEY_Q, KeyCode::KEY_RIGHTBRACE),
        (KeyCode::KEY_A, KeyCode::KEY_GRAVE),
        (KeyCode::KEY_BACKSLASH, KeyCode::KEY_SLASH),
    ];
    rows.into_iter()
        .flat_map(|(first, last)| (first.0..=last.0).map(KeyCode))
        .chain([KeyCode::KEY_SPACE])
        .chain(EXTRA_KEYS.iter().copied())
}

/// Several keys may type the same character; keep the simplest. The ISO key next to left
/// Shift (KEY_102ND) is missing on many keyboards, so it's a last resort.
pub fn keep_simplest(map: &mut HashMap<char, Combo>, c: char, combo: Combo) {
    let cost = |combo: &Combo| {
        let rare = combo.hold.iter().any(|k| EXTRA_KEYS.contains(k)) as usize;
        combo.hold.len() + combo.taps.len() * 2 + 10 * rare
    };
    if map.get(&c).is_none_or(|old| cost(&combo) < cost(old)) {
        map.insert(c, combo);
    }
}

impl Layout {
    pub fn resolve(&self, c: char) -> Option<&Combo> {
        self.chars.get(&c)
    }

    /// Built-in US layout, used when the platform's layout can't be read.
    pub fn us() -> Self {
        use KeyCode as K;
        let rows: &[(K, char, char)] = &[
            (K::KEY_GRAVE, '`', '~'),
            (K::KEY_1, '1', '!'),
            (K::KEY_2, '2', '@'),
            (K::KEY_3, '3', '#'),
            (K::KEY_4, '4', '$'),
            (K::KEY_5, '5', '%'),
            (K::KEY_6, '6', '^'),
            (K::KEY_7, '7', '&'),
            (K::KEY_8, '8', '*'),
            (K::KEY_9, '9', '('),
            (K::KEY_0, '0', ')'),
            (K::KEY_MINUS, '-', '_'),
            (K::KEY_EQUAL, '=', '+'),
            (K::KEY_LEFTBRACE, '[', '{'),
            (K::KEY_RIGHTBRACE, ']', '}'),
            (K::KEY_BACKSLASH, '\\', '|'),
            (K::KEY_SEMICOLON, ';', ':'),
            (K::KEY_APOSTROPHE, '\'', '"'),
            (K::KEY_COMMA, ',', '<'),
            (K::KEY_DOT, '.', '>'),
            (K::KEY_SLASH, '/', '?'),
            (K::KEY_Q, 'q', 'Q'),
            (K::KEY_W, 'w', 'W'),
            (K::KEY_E, 'e', 'E'),
            (K::KEY_R, 'r', 'R'),
            (K::KEY_T, 't', 'T'),
            (K::KEY_Y, 'y', 'Y'),
            (K::KEY_U, 'u', 'U'),
            (K::KEY_I, 'i', 'I'),
            (K::KEY_O, 'o', 'O'),
            (K::KEY_P, 'p', 'P'),
            (K::KEY_A, 'a', 'A'),
            (K::KEY_S, 's', 'S'),
            (K::KEY_D, 'd', 'D'),
            (K::KEY_F, 'f', 'F'),
            (K::KEY_G, 'g', 'G'),
            (K::KEY_H, 'h', 'H'),
            (K::KEY_J, 'j', 'J'),
            (K::KEY_K, 'k', 'K'),
            (K::KEY_L, 'l', 'L'),
            (K::KEY_Z, 'z', 'Z'),
            (K::KEY_X, 'x', 'X'),
            (K::KEY_C, 'c', 'C'),
            (K::KEY_V, 'v', 'V'),
            (K::KEY_B, 'b', 'B'),
            (K::KEY_N, 'n', 'N'),
            (K::KEY_M, 'm', 'M'),
        ];
        let mut chars = HashMap::new();
        chars.insert(' ', Combo::key(K::KEY_SPACE));
        for &(key, plain, shifted) in rows {
            chars.insert(plain, Combo::key(key));
            chars.insert(
                shifted,
                Combo {
                    taps: Vec::new(),
                    hold: vec![K::KEY_LEFTSHIFT, key],
                },
            );
        }
        Layout {
            name: LayoutName {
                layout: "us".into(),
                variant: String::new(),
            },
            chars,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn us_types_the_basics() {
        let us = Layout::us();
        assert_eq!(us.resolve('a'), Some(&Combo::key(KeyCode::KEY_A)));
        assert_eq!(
            us.resolve('!').map(|c| c.hold.clone()),
            Some(vec![KeyCode::KEY_LEFTSHIFT, KeyCode::KEY_1])
        );
    }
}
