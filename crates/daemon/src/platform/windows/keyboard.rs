//! Typing with `SendInput`. Keys go out as scancodes, which games reading raw keys see too;
//! characters the layout has no key for go out as text (`KEYEVENTF_UNICODE`).
//!
//! A held key repeats like on a keyboard. Windows leaves that to the keyboard's driver, so
//! keys typed with `SendInput` don't repeat by themselves: a thread here sends them again.

use std::io;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, KEYEVENTF_UNICODE, SendInput, VK_CAPITAL,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    SPI_GETKEYBOARDDELAY, SPI_GETKEYBOARDSPEED, SystemParametersInfoW,
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

pub struct Keyboard {
    repeat: Arc<Repeater>,
}

/// The key that repeats, shared with the thread that repeats it.
#[derive(Default)]
struct Repeater {
    held: Mutex<Option<Repeat>>,
    changed: Condvar,
}

struct Repeat {
    key: KeyCode,
    /// Its key-down event, sent again each time.
    input: INPUT,
    next: Instant,
    interval: Duration,
}

/// Watches the system Caps Lock state.
pub struct LedReader {
    last: Option<bool>,
}

impl Keyboard {
    pub fn new() -> io::Result<(Self, LedReader)> {
        let repeat = Arc::new(Repeater::default());
        let shared = repeat.clone();
        thread::spawn(move || repeat_keys(&shared));
        Ok((Keyboard { repeat }, LedReader { last: None }))
    }

    pub fn send(&mut self, events: &[(KeyCode, bool)]) -> io::Result<()> {
        // Held while sending, so no repeat lands between these events.
        let mut held = self.repeat.held.lock().unwrap();
        for &(k, down) in events {
            let Some(input) = input(k, down) else {
                continue;
            };
            // Like a keyboard: the last key pressed repeats until it's released.
            if down {
                *held = repeats(k).then(|| {
                    let (delay, interval) = repeat_timing();
                    Repeat {
                        key: k,
                        input,
                        next: Instant::now() + delay,
                        interval,
                    }
                });
            } else if held.as_ref().is_some_and(|r| r.key == k) {
                *held = None;
            }
            if !send_input(&input) {
                // Blocked by UIPI (an app running as administrator has focus): not fatal.
                eprintln!("typing failed: {}", io::Error::last_os_error());
                *held = None;
                break;
            }
        }
        self.repeat.changed.notify_one();
        Ok(())
    }
}

/// The key event for pressing or releasing `k`.
fn input(k: KeyCode, down: bool) -> Option<INPUT> {
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
}

/// Send one event: one at a time, so every app sees the presses in order and never merged.
/// False if Windows refused it.
fn send_input(input: &INPUT) -> bool {
    // SAFETY: one valid INPUT, with its size.
    unsafe { SendInput(1, input, size_of::<INPUT>() as i32) == 1 }
}

/// Whether holding `k` repeats it. Modifiers don't, and Caps Lock would toggle.
fn repeats(k: KeyCode) -> bool {
    ![
        KeyCode::KEY_LEFTSHIFT,
        KeyCode::KEY_RIGHTSHIFT,
        KeyCode::KEY_LEFTCTRL,
        KeyCode::KEY_RIGHTCTRL,
        KeyCode::KEY_LEFTALT,
        KeyCode::KEY_RIGHTALT,
        KeyCode::KEY_LEFTMETA,
        KeyCode::KEY_RIGHTMETA,
        KeyCode::KEY_CAPSLOCK,
    ]
    .contains(&k)
}

/// The repeat delay and interval set in Windows' keyboard settings, read at each press so
/// changes apply straight away.
fn repeat_timing() -> (Duration, Duration) {
    // Windows' defaults, in case the calls fail.
    let (mut delay, mut speed) = (1u32, 31u32);
    // SAFETY: each writes one integer to the pointer given.
    unsafe {
        SystemParametersInfoW(SPI_GETKEYBOARDDELAY, 0, (&mut delay as *mut u32).cast(), 0);
        SystemParametersInfoW(SPI_GETKEYBOARDSPEED, 0, (&mut speed as *mut u32).cast(), 0);
    }
    // Delay 0-3 is 250-1000 ms; speed 0-31 is about 2.5-30 repeats a second.
    let rate = 2.5 + speed.min(31) as f32 * 27.5 / 31.0;
    (
        Duration::from_millis(250 * (u64::from(delay.min(3)) + 1)),
        Duration::from_secs_f32(1.0 / rate),
    )
}

/// Send the held key's key-down again whenever it's due, until it's released.
fn repeat_keys(repeater: &Repeater) {
    let mut held = repeater.held.lock().unwrap();
    loop {
        let now = Instant::now();
        match held.as_ref().map(|r| r.next.saturating_duration_since(now)) {
            None => held = repeater.changed.wait(held).unwrap(),
            Some(wait) if !wait.is_zero() => {
                held = repeater.changed.wait_timeout(held, wait).unwrap().0;
            }
            Some(_) => {
                if let Some(r) = held.as_mut() {
                    r.next = now + r.interval;
                    let input = r.input;
                    if !send_input(&input) {
                        *held = None;
                    }
                }
            }
        }
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
