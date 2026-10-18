//! Character → key-press translation for the user's keyboard layout.
//!
//! The virtual keyboard can only send key positions; the desktop's layout decides which
//! character each position types. To type a given character we therefore ask libxkbcommon
//! (loaded at runtime, present on every Linux desktop) which key + modifiers produce it in
//! the active layout. If libxkbcommon or the layout can't be loaded we fall back to US.

use std::collections::HashMap;
use std::ffi::CString;
use std::process::Command;

use anyhow::{Context, Result, bail};
use evdev::KeyCode;
use xkbcommon_dl::{
    xkb_context_flags, xkb_key_direction, xkb_keymap_compile_flags, xkb_rule_names,
    xkbcommon_option,
};

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
}

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
    chars: HashMap<char, Combo>,
}

/// Modifier chords tried for each key, in order of preference.
const MODIFIER_CHORDS: &[&[KeyCode]] = &[
    &[],
    &[KeyCode::KEY_LEFTSHIFT],
    &[KeyCode::KEY_RIGHTALT],
    &[KeyCode::KEY_LEFTSHIFT, KeyCode::KEY_RIGHTALT],
];

/// Dead-key keysyms and the character they type when followed by space.
const DEAD_KEYS: &[(u32, char)] = &[
    (0xfe50, '`'),
    (0xfe51, '´'),
    (0xfe52, '^'),
    (0xfe53, '~'),
    (0xfe57, '¨'),
];

/// Keys that only exist on some keyboards (ISO, Japanese); used only if nothing else works.
const EXTRA_KEYS: &[KeyCode] = &[KeyCode::KEY_102ND, KeyCode::KEY_RO, KeyCode::KEY_YEN];

/// The main typing block. Keypad keys are excluded since they depend on NumLock.
fn typing_keys() -> impl Iterator<Item = KeyCode> {
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

/// evdev key codes are xkb key codes minus 8.
const XKB_OFFSET: u32 = 8;

impl Layout {
    /// The layout to use: `configured` if set, otherwise whatever the desktop uses.
    pub fn load(configured: Option<LayoutName>) -> Self {
        let name = configured.or_else(detect).unwrap_or_else(|| LayoutName {
            layout: "us".into(),
            variant: String::new(),
        });
        match Self::from_xkb(&name) {
            Ok(layout) => layout,
            Err(e) => {
                eprintln!("keyboard layout {name}: {e:#}; falling back to built-in US");
                Self::us()
            }
        }
    }

    pub fn resolve(&self, c: char) -> Option<&Combo> {
        self.chars.get(&c)
    }

    pub fn from_xkb(name: &LayoutName) -> Result<Self> {
        let xkb = xkbcommon_option().context("libxkbcommon not available")?;
        let layout = CString::new(name.layout.as_str())?;
        let variant = CString::new(name.variant.as_str())?;
        let (rules, model, options) = (
            CString::new("evdev")?,
            CString::new("pc105")?,
            CString::new("")?,
        );
        let names = xkb_rule_names {
            rules: rules.as_ptr(),
            model: model.as_ptr(),
            layout: layout.as_ptr(),
            variant: variant.as_ptr(),
            options: options.as_ptr(),
        };

        // SAFETY: straightforward use of the libxkbcommon C API; every object is unref'd
        // before returning and no pointer outlives this function.
        unsafe {
            let ctx = (xkb.xkb_context_new)(xkb_context_flags::XKB_CONTEXT_NO_FLAGS);
            if ctx.is_null() {
                bail!("xkb_context_new failed");
            }
            let keymap = (xkb.xkb_keymap_new_from_names)(
                ctx,
                &names,
                xkb_keymap_compile_flags::XKB_KEYMAP_COMPILE_NO_FLAGS,
            );
            if keymap.is_null() {
                (xkb.xkb_context_unref)(ctx);
                bail!("unknown layout");
            }
            let (min, max) = (
                (xkb.xkb_keymap_min_keycode)(keymap),
                (xkb.xkb_keymap_max_keycode)(keymap),
            );

            // Several keys may type the same character; keep the simplest. The ISO key next to
            // left Shift (KEY_102ND) is missing on many keyboards, so it's a last resort.
            let cost = |combo: &Combo| {
                let rare = combo.hold.iter().any(|k| EXTRA_KEYS.contains(k)) as usize;
                combo.hold.len() + combo.taps.len() * 2 + 10 * rare
            };
            let keep = |map: &mut HashMap<char, Combo>, c: char, combo: Combo| {
                if map.get(&c).is_none_or(|old| cost(&combo) < cost(old)) {
                    map.insert(c, combo);
                }
            };
            let mut direct = HashMap::new();
            let mut dead = HashMap::new();
            for chord in MODIFIER_CHORDS {
                let state = (xkb.xkb_state_new)(keymap);
                for m in chord.iter() {
                    (xkb.xkb_state_update_key)(
                        state,
                        m.0 as u32 + XKB_OFFSET,
                        xkb_key_direction::XKB_KEY_DOWN,
                    );
                }
                for key in typing_keys() {
                    let xkb_code = key.0 as u32 + XKB_OFFSET;
                    if xkb_code < min || xkb_code > max {
                        continue;
                    }
                    let mut hold = chord.to_vec();
                    hold.push(key);
                    let combo = Combo {
                        taps: Vec::new(),
                        hold,
                    };
                    if let Some(c) = char::from_u32((xkb.xkb_state_key_get_utf32)(state, xkb_code))
                        && !c.is_control()
                    {
                        keep(&mut direct, c, combo.clone());
                    }
                    let sym = (xkb.xkb_state_key_get_one_sym)(state, xkb_code);
                    if let Some(&(_, c)) = DEAD_KEYS.iter().find(|(s, _)| *s == sym) {
                        keep(&mut dead, c, combo);
                    }
                }
                (xkb.xkb_state_unref)(state);
            }
            (xkb.xkb_keymap_unref)(keymap);
            (xkb.xkb_context_unref)(ctx);

            // A dead key followed by space types the accent itself.
            for (c, dead_combo) in dead {
                direct.entry(c).or_insert(Combo {
                    taps: vec![dead_combo.hold],
                    hold: vec![KeyCode::KEY_SPACE],
                });
            }
            if !direct.contains_key(&'a') {
                bail!("layout has no 'a' key; refusing to use it");
            }
            Ok(Layout {
                name: name.clone(),
                chars: direct,
            })
        }
    }

    /// Built-in US layout, used when libxkbcommon is unavailable.
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

/// Find the desktop's keyboard layout: KDE, then GNOME, then the system default (localed).
/// Only the first layout is used when several are configured.
pub fn detect() -> Option<LayoutName> {
    detect_kde().or_else(detect_gnome).or_else(detect_localed)
}

fn first(list: &str) -> String {
    list.split(',').next().unwrap_or("").trim().to_owned()
}

fn detect_kde() -> Option<LayoutName> {
    let path = x360truewireless_common::config::config_dir()?.join("kxkbrc");
    let text = std::fs::read_to_string(path).ok()?;
    let get = |key: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(key)?.strip_prefix('=').map(first))
    };
    if get("Use").as_deref() == Some("false") {
        return None;
    }
    let layout = get("LayoutList").filter(|l| !l.is_empty())?;
    Some(LayoutName {
        layout,
        variant: get("VariantList").unwrap_or_default(),
    })
}

fn detect_gnome() -> Option<LayoutName> {
    // e.g. [('xkb', 'no'), ('xkb', 'us+dvorak')]
    let out = Command::new("gsettings")
        .args(["get", "org.gnome.desktop.input-sources", "sources"])
        .output()
        .ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    let start = text.find("('xkb', '")? + "('xkb', '".len();
    let value = &text[start..start + text[start..].find('\'')?];
    let (layout, variant) = value.split_once('+').unwrap_or((value, ""));
    Some(LayoutName {
        layout: layout.into(),
        variant: variant.into(),
    })
}

fn detect_localed() -> Option<LayoutName> {
    let out = Command::new("localectl").arg("status").output().ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    let get = |key: &str| {
        text.lines()
            .find_map(|l| l.trim().strip_prefix(key).map(|v| first(v.trim())))
    };
    let layout = get("X11 Layout:").filter(|l| !l.is_empty() && l != "n/a")?;
    Some(LayoutName {
        layout,
        variant: get("X11 Variant:").unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(name: &str, variant: &str) -> Option<Layout> {
        // Skip quietly on machines without libxkbcommon / xkb data.
        Layout::from_xkb(&LayoutName {
            layout: name.into(),
            variant: variant.into(),
        })
        .ok()
    }

    #[test]
    fn us_from_xkb_matches_builtin() {
        let Some(xkb) = layout("us", "") else { return };
        let builtin = Layout::us();
        for (c, combo) in &builtin.chars {
            assert_eq!(xkb.resolve(*c), Some(combo), "{c:?}");
        }
    }

    #[test]
    fn norwegian() {
        let Some(no) = layout("no", "") else { return };
        let k = |codes: &[KeyCode]| {
            Some(Combo {
                taps: Vec::new(),
                hold: codes.to_vec(),
            })
        };
        assert_eq!(
            no.resolve('@').cloned(),
            k(&[KeyCode::KEY_RIGHTALT, KeyCode::KEY_2])
        );
        assert_eq!(no.resolve('å').cloned(), k(&[KeyCode::KEY_LEFTBRACE]));
        assert_eq!(
            no.resolve('!').cloned(),
            k(&[KeyCode::KEY_LEFTSHIFT, KeyCode::KEY_1])
        );
        // '~' is a dead key on Norwegian: tap it, then space.
        let tilde = no.resolve('~').unwrap();
        assert_eq!(tilde.hold, vec![KeyCode::KEY_SPACE]);
        assert_eq!(tilde.taps.len(), 1);
    }
}
