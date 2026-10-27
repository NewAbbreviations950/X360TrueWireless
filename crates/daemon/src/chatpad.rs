//! Chatpad key reports → key events for the shared keyboard, with the one-shot modifiers and
//! the special combinations.

use x360truewireless_common::keycode::KeyCode;
use x360truewireless_proto::{ChatpadKey, ChatpadKeys, led, modifier};

use crate::keymap::{Keymap, Layer};

/// Shift, Green and Orange are one-shot like on the console: tap one, then the key. Holding
/// the modifier while pressing the key works too.
const STICKY: [u8; 3] = [modifier::SHIFT, modifier::GREEN, modifier::ORANGE];

/// What a chatpad report should do.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ChatpadOutput {
    /// Key events for the shared keyboard.
    pub keys: Vec<(KeyCode, bool)>,
    /// Press (`true`) or release the Steam Quick Access shortcut (Guide+A) on this controller's pad.
    pub quick_menu: Option<bool>,
    /// Caps Lock was toggled (Shift + Orange).
    pub caps_toggled: bool,
    /// Open Steam's friends & chat window (People).
    pub open_chat: bool,
    /// Chatpad keys newly pressed (for the settings app's "press a key" picker).
    pub pressed: Vec<ChatpadKey>,
}

/// Turns one chatpad's key reports into key events for the shared keyboard.
///
/// Special combinations (either key may be held or tapped first):
/// - Shift + Orange toggles Caps Lock.
/// - Green + People opens the Steam Quick Access menu; People alone opens Steam chat.
#[derive(Default)]
pub struct ChatpadState {
    modifiers: u8,
    /// One-shot modifiers waiting for the next key.
    latched: u8,
    /// Held modifiers that were used for something (so releasing them doesn't latch).
    used: u8,
    /// Whether the held People key is acting as the Quick Access shortcut rather than Meta.
    people_quick_menu: bool,
    /// Raw codes currently held, with the chord held for each (released in reverse).
    held: Vec<(u8, Vec<KeyCode>)>,
}

impl ChatpadState {
    pub fn update(&mut self, report: ChatpadKeys, keymap: &Keymap) -> ChatpadOutput {
        let mut out = ChatpadOutput::default();
        let pressed = report.modifiers & !self.modifiers;
        let released = self.modifiers & !report.modifiers;
        self.used &= !pressed;

        if pressed & modifier::PEOPLE != 0 {
            let green = (report.modifiers | self.latched) & modifier::GREEN;
            self.people_quick_menu = green != 0;
            if self.people_quick_menu {
                self.latched &= !modifier::GREEN;
                self.used |= green & report.modifiers;
                out.quick_menu = Some(true);
            } else {
                out.open_chat = true;
            }
        }
        if released & modifier::PEOPLE != 0 && self.people_quick_menu {
            out.quick_menu = Some(false);
        }

        for bit in STICKY {
            if released & bit == 0 || self.used & bit != 0 {
                continue;
            }
            let caps_partner = match bit {
                modifier::SHIFT => modifier::ORANGE,
                modifier::ORANGE => modifier::SHIFT,
                _ => 0,
            };
            if (report.modifiers | self.latched) & caps_partner != 0 {
                out.keys.extend([
                    (KeyCode::KEY_CAPSLOCK, true),
                    (KeyCode::KEY_CAPSLOCK, false),
                ]);
                out.caps_toggled = true;
                self.latched &= !(bit | caps_partner);
                self.used |= caps_partner & report.modifiers;
                continue;
            }
            self.latched ^= bit;
            // Green and Orange are alternatives; latching one cancels the other.
            if bit != modifier::SHIFT && self.latched & bit != 0 {
                self.latched &= !(modifier::GREEN | modifier::ORANGE) | bit;
            }
        }
        self.modifiers = report.modifiers;

        // Release keys no longer reported.
        self.held.retain(|(code, chord)| {
            if report.keys.contains(code) {
                return true;
            }
            out.keys.extend(chord.iter().rev().map(|&k| (k, false)));
            false
        });

        for &code in report.keys.iter().filter(|&&c| c != 0) {
            if self.held.iter().any(|(c, _)| *c == code) {
                continue;
            }
            let active = report.modifiers | self.latched;
            let layer = if active & modifier::GREEN != 0 {
                Layer::Green
            } else if active & modifier::ORANGE != 0 {
                Layer::Orange
            } else {
                Layer::Normal
            };
            let combo = match ChatpadKey::from_code(code) {
                Some(key) => {
                    out.pressed.push(key);
                    keymap.combo(layer, key, active & modifier::SHIFT != 0)
                }
                None => {
                    eprintln!("unknown chatpad key code {code:#04x}");
                    None
                }
            };
            let chord = match combo {
                Some(combo) => {
                    for tap in &combo.taps {
                        out.keys.extend(tap.iter().map(|&k| (k, true)));
                        out.keys.extend(tap.iter().rev().map(|&k| (k, false)));
                    }
                    out.keys.extend(combo.hold.iter().map(|&k| (k, true)));
                    combo.hold
                }
                None => Vec::new(),
            };
            self.held.push((code, chord));
            self.latched = 0;
            self.used |= report.modifiers;
        }
        out
    }

    /// Indicator lights (`led::*` bits) this state should show: Caps for the system's Caps
    /// Lock or a latched Shift, Green/Orange while latched or held.
    pub fn indicators(&self, caps_lock: bool) -> u8 {
        let active = self.modifiers | self.latched;
        let mut leds = 0;
        if caps_lock || self.latched & modifier::SHIFT != 0 {
            leds |= led::CAPS;
        }
        if active & modifier::GREEN != 0 {
            leds |= led::GREEN;
        }
        if active & modifier::ORANGE != 0 {
            leds |= led::ORANGE;
        }
        leds
    }

    /// Release everything (controller disconnected or daemon stopping).
    pub fn release_all(&mut self, keymap: &Keymap) -> ChatpadOutput {
        let out = self.update(ChatpadKeys::default(), keymap);
        self.latched = 0;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Layout;
    use KeyCode as K;

    fn report(modifiers: u8, keys: &[u8]) -> ChatpadKeys {
        let mut r = ChatpadKeys {
            modifiers,
            keys: [0; 2],
        };
        r.keys[..keys.len()].copy_from_slice(keys);
        r
    }

    const A: u8 = 0x37;
    const Q: u8 = 0x27;

    #[test]
    fn plain_key() {
        let map = Keymap::new(Layout::us());
        let mut s = ChatpadState::default();
        assert_eq!(s.update(report(0, &[A]), &map).keys, vec![(K::KEY_A, true)]);
        assert_eq!(s.update(report(0, &[]), &map).keys, vec![(K::KEY_A, false)]);
    }

    #[test]
    fn held_shift() {
        let map = Keymap::new(Layout::us());
        let mut s = ChatpadState::default();
        assert!(s.update(report(modifier::SHIFT, &[]), &map).keys.is_empty());
        assert_eq!(
            s.update(report(modifier::SHIFT, &[A]), &map).keys,
            vec![(K::KEY_LEFTSHIFT, true), (K::KEY_A, true)]
        );
        s.update(report(modifier::SHIFT, &[]), &map);
        s.update(report(0, &[]), &map);
        // Shift was used while held, so it must not latch for the next key.
        assert_eq!(s.update(report(0, &[A]), &map).keys, vec![(K::KEY_A, true)]);
    }

    #[test]
    fn tapped_green_applies_once() {
        let map = Keymap::new(Layout::us());
        let mut s = ChatpadState::default();
        s.update(report(modifier::GREEN, &[]), &map);
        s.update(report(0, &[]), &map);
        // Green+Q = '!'
        assert_eq!(
            s.update(report(0, &[Q]), &map).keys,
            vec![(K::KEY_LEFTSHIFT, true), (K::KEY_1, true)]
        );
        assert_eq!(
            s.update(report(0, &[]), &map).keys,
            vec![(K::KEY_1, false), (K::KEY_LEFTSHIFT, false)]
        );
        assert_eq!(s.update(report(0, &[Q]), &map).keys, vec![(K::KEY_Q, true)]);
    }

    #[test]
    fn tapping_green_twice_cancels() {
        let map = Keymap::new(Layout::us());
        let mut s = ChatpadState::default();
        for _ in 0..2 {
            s.update(report(modifier::GREEN, &[]), &map);
            s.update(report(0, &[]), &map);
        }
        assert_eq!(s.update(report(0, &[Q]), &map).keys, vec![(K::KEY_Q, true)]);
    }

    #[test]
    fn orange_replaces_latched_green() {
        let map = Keymap::new(Layout::us());
        let mut s = ChatpadState::default();
        for m in [modifier::GREEN, modifier::ORANGE] {
            s.update(report(m, &[]), &map);
            s.update(report(0, &[]), &map);
        }
        // Orange+R = '$'
        assert_eq!(
            s.update(report(0, &[0x24]), &map).keys,
            vec![(K::KEY_LEFTSHIFT, true), (K::KEY_4, true)]
        );
    }

    const CAPS: [(KeyCode, bool); 2] = [(K::KEY_CAPSLOCK, true), (K::KEY_CAPSLOCK, false)];

    #[test]
    fn shift_plus_orange_toggles_caps_lock() {
        let map = Keymap::new(Layout::us());
        // Hold Shift, tap Orange.
        let mut s = ChatpadState::default();
        s.update(report(modifier::SHIFT, &[]), &map);
        s.update(report(modifier::SHIFT | modifier::ORANGE, &[]), &map);
        assert_eq!(s.update(report(modifier::SHIFT, &[]), &map).keys, CAPS);
        s.update(report(0, &[]), &map);
        // Nothing left latched: the next key is plain.
        assert_eq!(s.update(report(0, &[A]), &map).keys, vec![(K::KEY_A, true)]);

        // Tap Orange, then tap Shift.
        let mut s = ChatpadState::default();
        s.update(report(modifier::ORANGE, &[]), &map);
        s.update(report(0, &[]), &map);
        s.update(report(modifier::SHIFT, &[]), &map);
        assert_eq!(s.update(report(0, &[]), &map).keys, CAPS);
        assert_eq!(s.update(report(0, &[A]), &map).keys, vec![(K::KEY_A, true)]);
    }

    #[test]
    fn green_plus_people_is_quick_menu() {
        let map = Keymap::new(Layout::us());
        let mut s = ChatpadState::default();
        s.update(report(modifier::GREEN, &[]), &map);
        let down = s.update(report(modifier::GREEN | modifier::PEOPLE, &[]), &map);
        assert_eq!(
            down,
            ChatpadOutput {
                quick_menu: Some(true),
                ..Default::default()
            }
        );
        let up = s.update(report(modifier::GREEN, &[]), &map);
        assert_eq!(
            up,
            ChatpadOutput {
                quick_menu: Some(false),
                ..Default::default()
            }
        );
        // Green was used, so releasing it doesn't latch.
        s.update(report(0, &[]), &map);
        assert_eq!(s.update(report(0, &[Q]), &map).keys, vec![(K::KEY_Q, true)]);
    }

    #[test]
    fn tapped_green_then_people_is_quick_menu() {
        let map = Keymap::new(Layout::us());
        let mut s = ChatpadState::default();
        s.update(report(modifier::GREEN, &[]), &map);
        s.update(report(0, &[]), &map);
        assert_eq!(
            s.update(report(modifier::PEOPLE, &[]), &map).quick_menu,
            Some(true)
        );
        assert_eq!(s.update(report(0, &[]), &map).quick_menu, Some(false));
    }

    #[test]
    fn people_alone_opens_chat() {
        let map = Keymap::new(Layout::us());
        let mut s = ChatpadState::default();
        let down = s.update(report(modifier::PEOPLE, &[]), &map);
        assert!(down.open_chat && down.keys.is_empty() && down.quick_menu.is_none());
        let up = s.update(report(0, &[]), &map);
        assert_eq!(up, ChatpadOutput::default());
    }

    #[test]
    fn indicators_follow_latches_and_caps() {
        let map = Keymap::new(Layout::us());
        let mut s = ChatpadState::default();
        s.update(report(modifier::GREEN, &[]), &map);
        assert_eq!(s.indicators(false), led::GREEN); // held
        s.update(report(0, &[]), &map);
        assert_eq!(s.indicators(false), led::GREEN); // latched
        s.update(report(0, &[Q]), &map);
        assert_eq!(s.indicators(false), 0); // consumed
        assert_eq!(s.indicators(true), led::CAPS); // system Caps Lock
        s.update(report(0, &[]), &map);
        s.update(report(modifier::SHIFT, &[]), &map);
        s.update(report(modifier::SHIFT | modifier::ORANGE, &[]), &map);
        assert!(s.update(report(modifier::SHIFT, &[]), &map).caps_toggled);
        s.update(report(0, &[]), &map);
        // Latched Shift lights Caps until used.
        s.update(report(modifier::SHIFT, &[]), &map);
        s.update(report(0, &[]), &map);
        assert_eq!(s.indicators(false), led::CAPS);
    }
}
