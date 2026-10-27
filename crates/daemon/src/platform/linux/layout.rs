//! Keyboard layouts through libxkbcommon (loaded at runtime, present on every Linux desktop),
//! and finding the desktop's layout.

use std::collections::HashMap;
use std::ffi::CString;
use std::process::Command;

use anyhow::{Context, Result, bail};
use x360truewireless_common::keycode::KeyCode;
use xkbcommon_dl::{
    xkb_context_flags, xkb_key_direction, xkb_keymap_compile_flags, xkb_rule_names,
    xkbcommon_option,
};

use crate::layout::{
    Combo, Layout, LayoutName, MODIFIER_CHORDS, keep_simplest as keep, typing_keys,
};

/// Dead-key keysyms and the character they type when followed by space.
const DEAD_KEYS: &[(u32, char)] = &[
    (0xfe50, '`'),
    (0xfe51, '´'),
    (0xfe52, '^'),
    (0xfe53, '~'),
    (0xfe57, '¨'),
];

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
}

/// Whether the layout needs re-detecting before typing. On Linux that takes a reload.
pub fn changed() -> bool {
    false
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
