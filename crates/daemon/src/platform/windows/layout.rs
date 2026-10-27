//! Keyboard layouts on Windows: the active layout of the window in front, read key by key
//! with `ToUnicodeEx`. Windows keeps a layout per window, so it's checked again whenever the
//! chatpad types.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use windows_sys::Win32::Globalization::LCIDToLocaleName;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyboardLayout, HKL, MAPVK_VSC_TO_VK_EX, MapVirtualKeyExW, ToUnicodeEx, VK_CONTROL,
    VK_LCONTROL, VK_LSHIFT, VK_MENU, VK_RMENU, VK_SHIFT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
use x360truewireless_common::keycode::KeyCode;

use super::keyboard::{Key, key};
use crate::layout::{Combo, Layout, LayoutName, MODIFIER_CHORDS, keep_simplest, typing_keys};

/// The layout the table was last built for.
static CURRENT: AtomicUsize = AtomicUsize::new(0);

fn foreground_layout() -> HKL {
    // SAFETY: plain calls; a null window gives thread 0, the calling thread's layout.
    unsafe {
        let thread = GetWindowThreadProcessId(GetForegroundWindow(), std::ptr::null_mut());
        GetKeyboardLayout(thread)
    }
}

/// Whether the window in front uses a different layout than the table was built for.
pub fn changed() -> bool {
    foreground_layout() as usize != CURRENT.load(Ordering::SeqCst)
}

/// The layout's language, like `nb-NO`.
fn name(hkl: HKL) -> LayoutName {
    let lcid = hkl as usize as u32 & 0xffff;
    let mut buf = [0u16; 85];
    // SAFETY: `buf` is LOCALE_NAME_MAX_LENGTH long.
    let n = unsafe { LCIDToLocaleName(lcid, buf.as_mut_ptr(), buf.len() as i32, 0) };
    let layout = if n > 1 {
        String::from_utf16_lossy(&buf[..n as usize - 1])
    } else {
        format!("{:08x}", hkl as usize)
    };
    LayoutName {
        layout,
        variant: String::new(),
    }
}

impl Layout {
    /// The layout of the window in front. A layout set in config.toml is for Linux's layout
    /// names, so it's not used here.
    pub fn load(_configured: Option<LayoutName>) -> Self {
        let hkl = foreground_layout();
        CURRENT.store(hkl as usize, Ordering::SeqCst);
        let layout = Self::from_hkl(hkl);
        if layout.resolve('a').is_none() {
            eprintln!("keyboard layout {} has no 'a' key; using US", layout.name);
            return Self::us();
        }
        layout
    }

    fn from_hkl(hkl: HKL) -> Self {
        let mut direct = HashMap::new();
        let mut dead = HashMap::new();
        for chord in MODIFIER_CHORDS {
            let mut state = [0u8; 256];
            for &m in chord.iter() {
                let vks: &[u16] = if m == KeyCode::KEY_LEFTSHIFT {
                    &[VK_SHIFT, VK_LSHIFT]
                } else {
                    // AltGr is Ctrl+Alt to Windows.
                    &[VK_CONTROL, VK_LCONTROL, VK_MENU, VK_RMENU]
                };
                for &vk in vks {
                    state[vk as usize] = 0x80;
                }
            }
            for k in typing_keys() {
                let Some(Key::Scan { code, .. }) = key(k) else {
                    continue;
                };
                // SAFETY: plain calls; `state` and `buf` are the sizes ToUnicodeEx expects.
                let (n, buf) = unsafe {
                    let vk = MapVirtualKeyExW(code as u32, MAPVK_VSC_TO_VK_EX, hkl);
                    if vk == 0 {
                        continue;
                    }
                    let mut buf = [0u16; 8];
                    // Flag 4: leave the keyboard's dead-key state alone.
                    let n = ToUnicodeEx(
                        vk,
                        code as u32,
                        state.as_ptr(),
                        buf.as_mut_ptr(),
                        buf.len() as i32,
                        4,
                        hkl,
                    );
                    (n, buf)
                };
                let mut hold = chord.to_vec();
                hold.push(k);
                let combo = Combo {
                    taps: Vec::new(),
                    hold,
                };
                let single = |len: usize| {
                    char::decode_utf16(buf[..len].iter().copied())
                        .next()
                        .and_then(Result::ok)
                        .filter(|c| !c.is_control())
                };
                match n {
                    1 => {
                        if let Some(c) = single(1) {
                            keep_simplest(&mut direct, c, combo);
                        }
                    }
                    // A dead key: the buffer holds its spacing form.
                    n if n < 0 => {
                        if let Some(c) = single(1) {
                            keep_simplest(&mut dead, c, combo);
                        }
                    }
                    _ => {}
                }
            }
        }
        // A dead key followed by space types the accent itself.
        for (c, dead_combo) in dead {
            direct.entry(c).or_insert(Combo {
                taps: vec![dead_combo.hold],
                hold: vec![KeyCode::KEY_SPACE],
            });
        }
        Layout {
            name: name(hkl),
            chars: direct,
        }
    }
}
