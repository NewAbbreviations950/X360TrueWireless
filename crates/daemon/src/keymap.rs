//! What each chatpad key types: the key table (defaults + config overrides, see
//! `x360truewireless_common::keymap`) turned into key presses through the user's keyboard layout
//! (see `layout.rs`).

use x360truewireless_common::keycode::KeyCode;
use x360truewireless_common::keymap::{KeyTable, Output};
use x360truewireless_proto::ChatpadKey;

pub use x360truewireless_common::keymap::Layer;

use crate::layout::{Combo, Layout};

pub struct Keymap {
    table: KeyTable,
    layout: Layout,
}

impl Keymap {
    #[cfg(test)]
    pub fn new(layout: Layout) -> Self {
        Self::with_table(layout, KeyTable::default())
    }

    pub fn with_table(layout: Layout, table: KeyTable) -> Self {
        Keymap { table, layout }
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// Key presses for `key` on `layer`. `shift` adds Shift on the normal layer (so letters
    /// become capitals); alternate-layer characters are typed exactly as printed.
    pub fn combo(&self, layer: Layer, key: ChatpadKey, shift: bool) -> Option<Combo> {
        let mut combo = match self.table.output(layer, key)? {
            Output::Char(c) => match self.layout.resolve(c) {
                Some(combo) => combo.clone(),
                #[cfg(windows)]
                None => Combo::text(c)?,
                #[cfg(not(windows))]
                None => {
                    eprintln!(
                        "{c:?} can't be typed with keyboard layout {}",
                        self.layout.name
                    );
                    return None;
                }
            },
            Output::Key(k) => Combo::key(k),
            Output::Nothing => return None,
        };
        let uses_layer = layer != Layer::Normal && self.table.defines(layer, key);
        if shift && !uses_layer && !combo.hold.contains(&KeyCode::KEY_LEFTSHIFT) {
            combo.hold.insert(0, KeyCode::KEY_LEFTSHIFT);
        }
        Some(combo)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_types_something_on_us() {
        let map = Keymap::new(Layout::us());
        for &k in ChatpadKey::ALL {
            assert!(map.combo(Layer::Normal, k, false).is_some(), "{k:?}");
        }
    }

    #[test]
    fn shift_capitalises_but_not_layer_symbols() {
        let map = Keymap::new(Layout::us());
        let shifted_a = map.combo(Layer::Normal, ChatpadKey::A, true).unwrap();
        assert_eq!(shifted_a.hold, vec![KeyCode::KEY_LEFTSHIFT, KeyCode::KEY_A]);
        // Green+H is '/', which needs no shift; chatpad Shift must not turn it into '?'.
        let slash = map.combo(Layer::Green, ChatpadKey::H, true).unwrap();
        assert_eq!(slash.hold, vec![KeyCode::KEY_SLASH]);
    }

    #[test]
    fn unmapped_layer_key_falls_back_to_normal() {
        let map = Keymap::new(Layout::us());
        assert_eq!(
            map.combo(Layer::Green, ChatpadKey::Space, false)
                .unwrap()
                .hold,
            vec![KeyCode::KEY_SPACE]
        );
    }

    #[test]
    fn disabled_key_types_nothing() {
        use x360truewireless_common::keymap::Overrides;
        let mut o = Overrides::new();
        o.entry(Layer::Normal)
            .or_default()
            .insert("Q".into(), String::new());
        let map = Keymap::with_table(Layout::us(), KeyTable::with_overrides(&o).0);
        assert_eq!(map.combo(Layer::Normal, ChatpadKey::Q, false), None);
    }
}
