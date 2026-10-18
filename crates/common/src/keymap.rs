//! The chatpad keymap: what each key types on each layer, with the defaults and the
//! user's overrides from `config.toml`.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::str::FromStr;

use evdev::KeyCode;
use serde::{Deserialize, Serialize};
use x360truewireless_proto::ChatpadKey;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Layer {
    Normal,
    Green,
    Orange,
}

impl Layer {
    pub const ALL: [Layer; 3] = [Layer::Normal, Layer::Green, Layer::Orange];

    pub fn name(self) -> &'static str {
        match self {
            Layer::Normal => "normal",
            Layer::Green => "green",
            Layer::Orange => "orange",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Layer::ALL.into_iter().find(|l| l.name() == name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Output {
    /// Type this character, whatever keys that takes in the current layout.
    Char(char),
    /// Press this key as-is (Enter, arrows, F-keys, …).
    Key(KeyCode),
    /// Do nothing.
    Nothing,
}

/// Config-file spelling: a single character (`"!"`), an evdev key name (`"KEY_F5"`), or `""`.
impl fmt::Display for Output {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Output::Char(c) => write!(f, "{c}"),
            Output::Key(k) => write!(f, "{k:?}"),
            Output::Nothing => Ok(()),
        }
    }
}

impl FromStr for Output {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        let mut chars = s.chars();
        match (chars.next(), chars.next()) {
            (None, _) => Ok(Output::Nothing),
            (Some(c), None) => Ok(Output::Char(c)),
            _ => KeyCode::from_str(s).map(Output::Key).map_err(|_| {
                format!("{s:?} is neither a single character nor a key name like KEY_ENTER")
            }),
        }
    }
}

/// Per-layer overrides as stored in the config: chatpad key name → output spelling.
pub type Overrides = BTreeMap<Layer, BTreeMap<String, String>>;

#[derive(Debug, Clone)]
pub struct KeyTable {
    entries: HashMap<(Layer, ChatpadKey), Output>,
}

impl Default for KeyTable {
    fn default() -> Self {
        let mut entries = HashMap::new();
        for (layer, table) in [
            (Layer::Normal, NORMAL),
            (Layer::Green, GREEN),
            (Layer::Orange, ORANGE),
        ] {
            for &(key, out) in table {
                entries.insert((layer, key), out);
            }
        }
        KeyTable { entries }
    }
}

impl KeyTable {
    /// Defaults with `overrides` applied. Invalid entries are skipped and reported.
    pub fn with_overrides(overrides: &Overrides) -> (Self, Vec<String>) {
        let mut table = Self::default();
        let mut errors = Vec::new();
        for (&layer, keys) in overrides {
            for (name, value) in keys {
                let Some(key) = ChatpadKey::from_name(name) else {
                    errors.push(format!("[{}] unknown chatpad key {name:?}", layer.name()));
                    continue;
                };
                match value.parse() {
                    Ok(out) => {
                        table.entries.insert((layer, key), out);
                    }
                    Err(e) => errors.push(format!("[{}] {name}: {e}", layer.name())),
                }
            }
        }
        (table, errors)
    }

    /// Output for `key` on `layer`, falling back to the normal layer if the layer has none.
    pub fn output(&self, layer: Layer, key: ChatpadKey) -> Option<Output> {
        self.entries
            .get(&(layer, key))
            .or_else(|| self.entries.get(&(Layer::Normal, key)))
            .copied()
    }

    /// Whether `layer` defines `key` itself (rather than falling back to normal).
    pub fn defines(&self, layer: Layer, key: ChatpadKey) -> bool {
        self.entries.contains_key(&(layer, key))
    }

    /// The built-in default for `key` on `layer`, if any (no fallback).
    pub fn default_output(layer: Layer, key: ChatpadKey) -> Option<Output> {
        let table = match layer {
            Layer::Normal => NORMAL,
            Layer::Green => GREEN,
            Layer::Orange => ORANGE,
        };
        table.iter().find(|(k, _)| *k == key).map(|&(_, out)| out)
    }
}

use ChatpadKey as C;
use Output::{Char as Ch, Key};

pub const NORMAL: &[(ChatpadKey, Output)] = &[
    (C::D1, Ch('1')),
    (C::D2, Ch('2')),
    (C::D3, Ch('3')),
    (C::D4, Ch('4')),
    (C::D5, Ch('5')),
    (C::D6, Ch('6')),
    (C::D7, Ch('7')),
    (C::D8, Ch('8')),
    (C::D9, Ch('9')),
    (C::D0, Ch('0')),
    (C::Q, Ch('q')),
    (C::W, Ch('w')),
    (C::E, Ch('e')),
    (C::R, Ch('r')),
    (C::T, Ch('t')),
    (C::Y, Ch('y')),
    (C::U, Ch('u')),
    (C::I, Ch('i')),
    (C::O, Ch('o')),
    (C::P, Ch('p')),
    (C::A, Ch('a')),
    (C::S, Ch('s')),
    (C::D, Ch('d')),
    (C::F, Ch('f')),
    (C::G, Ch('g')),
    (C::H, Ch('h')),
    (C::J, Ch('j')),
    (C::K, Ch('k')),
    (C::L, Ch('l')),
    (C::Comma, Ch(',')),
    (C::Z, Ch('z')),
    (C::X, Ch('x')),
    (C::C, Ch('c')),
    (C::V, Ch('v')),
    (C::B, Ch('b')),
    (C::N, Ch('n')),
    (C::M, Ch('m')),
    (C::Period, Ch('.')),
    (C::Enter, Key(KeyCode::KEY_ENTER)),
    (C::Backspace, Key(KeyCode::KEY_BACKSPACE)),
    (C::Space, Key(KeyCode::KEY_SPACE)),
    (C::Left, Key(KeyCode::KEY_LEFT)),
    (C::Right, Key(KeyCode::KEY_RIGHT)),
];

// Legends as printed on a US/English 360 chatpad.
pub const GREEN: &[(ChatpadKey, Output)] = &[
    (C::Q, Ch('!')),
    (C::W, Ch('@')),
    (C::E, Ch('€')),
    (C::R, Ch('#')),
    (C::T, Ch('%')),
    (C::Y, Ch('^')),
    (C::U, Ch('&')),
    (C::I, Ch('*')),
    (C::O, Ch('(')),
    (C::P, Ch(')')),
    (C::A, Ch('~')),
    (C::S, Ch('š')),
    (C::D, Ch('{')),
    (C::F, Ch('}')),
    (C::G, Ch('¨')),
    (C::H, Ch('/')),
    (C::J, Ch('\'')),
    (C::K, Ch('[')),
    (C::L, Ch(']')),
    (C::Comma, Ch(':')),
    (C::Z, Ch('`')),
    (C::X, Ch('«')),
    (C::C, Ch('»')),
    (C::V, Ch('-')),
    (C::B, Ch('|')),
    (C::N, Ch('<')),
    (C::M, Ch('>')),
    (C::Period, Ch('?')),
];

pub const ORANGE: &[(ChatpadKey, Output)] = &[
    (C::Q, Ch('¡')),
    (C::W, Ch('å')),
    (C::E, Ch('è')),
    (C::R, Ch('$')),
    (C::T, Ch('þ')),
    (C::Y, Ch('ý')),
    (C::U, Ch('ú')),
    (C::I, Ch('ì')),
    (C::O, Ch('ò')),
    (C::P, Ch('=')),
    (C::A, Ch('á')),
    (C::S, Ch('ß')),
    (C::D, Ch('ð')),
    (C::F, Ch('£')),
    (C::G, Ch('¥')),
    (C::H, Ch('\\')),
    (C::J, Ch('"')),
    (C::L, Ch('ø')),
    (C::Comma, Ch(';')),
    (C::Z, Ch('æ')),
    (C::X, Ch('œ')),
    (C::C, Ch('ç')),
    (C::V, Ch('_')),
    (C::B, Ch('+')),
    (C::N, Ch('ñ')),
    (C::M, Ch('µ')),
    (C::Period, Ch('¿')),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_spelling_roundtrips() {
        for s in ["!", "å", "KEY_F5", "KEY_ENTER", ""] {
            assert_eq!(s.parse::<Output>().unwrap().to_string(), s);
        }
        assert!("ab".parse::<Output>().is_err());
    }

    #[test]
    fn overrides_apply_and_report_errors() {
        let mut o = Overrides::new();
        o.entry(Layer::Green)
            .or_default()
            .insert("Q".into(), "KEY_F1".into());
        o.entry(Layer::Green)
            .or_default()
            .insert("Nope".into(), "x".into());
        let (table, errors) = KeyTable::with_overrides(&o);
        assert_eq!(
            table.output(Layer::Green, ChatpadKey::Q),
            Some(Output::Key(KeyCode::KEY_F1))
        );
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn layer_falls_back_to_normal() {
        let t = KeyTable::default();
        assert_eq!(
            t.output(Layer::Green, ChatpadKey::Space),
            Some(Output::Key(KeyCode::KEY_SPACE))
        );
        assert!(!t.defines(Layer::Green, ChatpadKey::Space));
    }
}
