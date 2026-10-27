//! Typing with `SendInput`. Keys go out as scancodes, which games reading raw keys see too;
//! characters the layout has no key for go out as text (`KEYEVENTF_UNICODE`).

use std::io;
use std::thread;
use std::time::Duration;

use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, KEYEVENTF_UNICODE, SendInput, VK_CAPITAL,
};
use x360truewireless_common::keycode::KeyCode;

use crate::layout::TEXT_BASE;

/// How a key goes out: a scancode (extended or not), or a virtual key for keys without a
/// fixed scancode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Scan { code: u16, extended: bool },
    Virtual(u16),
}

/// Linux key codes 1–88 are the PC (set 1) scancodes; the rest need a table.
pub fn key(k: KeyCode) -> Option<Key> {
    let scan = |code| {
        Some(Key::Scan {
            code,
            extended: false,
        })
    };
    let extended = |code| {
        Some(Key::Scan {
            code,
            extended: true,
        })
    };
    let n = k.0;
    match n {
        1..=83 | 86..=88 => scan(n),
        89 => scan(0x73),  // KEY_RO
        124 => scan(0x7d), // KEY_YEN
        96 => extended(0x1c),
        97 => extended(0x1d),
        98 => extended(0x35),
        99 => extended(0x37),
        100 => extended(0x38),
        102 => extended(0x47),
        103 => extended(0x48),
        104 => extended(0x49),
        105 => extended(0x4b),
        106 => extended(0x4d),
        107 => extended(0x4f),
        108 => extended(0x50),
        109 => extended(0x51),
        110 => extended(0x52),
        111 => extended(0x53),
        125 => extended(0x5b),
        126 => extended(0x5c),
        127 => extended(0x5d),
        // Virtual keys from WinUser.h.
        113 => Some(Key::Virtual(0xad)),                 // mute
        114 => Some(Key::Virtual(0xae)),                 // volume down
        115 => Some(Key::Virtual(0xaf)),                 // volume up
        119 => Some(Key::Virtual(0x13)),                 // pause
        158 => Some(Key::Virtual(0xa6)),                 // browser back
        159 => Some(Key::Virtual(0xa7)),                 // browser forward
        163 => Some(Key::Virtual(0xb0)),                 // next track
        164 => Some(Key::Virtual(0xb3)),                 // play/pause
        165 => Some(Key::Virtual(0xb1)),                 // previous track
        166 => Some(Key::Virtual(0xb2)),                 // stop
        183..=194 => Some(Key::Virtual(0x7c + n - 183)), // F13–F24
        _ => None,
    }
}

pub struct Keyboard;

/// Watches the system Caps Lock state.
pub struct LedReader {
    last: Option<bool>,
}

impl Keyboard {
    pub fn new() -> io::Result<(Self, LedReader)> {
        Ok((Keyboard, LedReader { last: None }))
    }

    pub fn send(&mut self, events: &[(KeyCode, bool)]) -> io::Result<()> {
        let inputs: Vec<INPUT> = events
            .iter()
            .filter_map(|&(k, down)| {
                let up = if down { 0 } else { KEYEVENTF_KEYUP };
                let (vk, scan, flags) = if k.0 >= TEXT_BASE {
                    (0, k.0 - TEXT_BASE, KEYEVENTF_UNICODE)
                } else {
                    match key(k) {
                        Some(Key::Scan { code, extended }) => (
                            0,
                            code,
                            KEYEVENTF_SCANCODE | if extended { KEYEVENTF_EXTENDEDKEY } else { 0 },
                        ),
                        Some(Key::Virtual(vk)) => (vk, 0, 0),
                        None => {
                            eprintln!("{k:?} can't be pressed on Windows");
                            return None;
                        }
                    }
                };
                Some(INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: vk,
                            wScan: scan,
                            dwFlags: flags | up,
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                })
            })
            .collect();
        // One at a time, so every app sees the presses in order and never merged.
        for input in &inputs {
            // SAFETY: one valid INPUT, with its size.
            if unsafe { SendInput(1, input, size_of::<INPUT>() as i32) } != 1 {
                // Blocked by UIPI (an app running as administrator has focus): not fatal.
                eprintln!("typing failed: {}", io::Error::last_os_error());
                break;
            }
        }
        Ok(())
    }
}

impl LedReader {
    /// Block until Caps Lock changes; returns its new state.
    pub fn next_caps_lock(&mut self) -> io::Result<bool> {
        loop {
            // SAFETY: plain call; the low bit is the toggle state.
            let on = unsafe { GetKeyState(VK_CAPITAL as i32) } & 1 != 0;
            if self.last != Some(on) {
                self.last = Some(on);
                return Ok(on);
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
}
