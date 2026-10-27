//! The virtual keyboard all chatpads type into.
//!
//! Built directly on uinput (rather than the `evdev` crate's builder) so it can declare a
//! Caps Lock LED. The compositor then mirrors the system Caps Lock state onto it, as it does
//! for every keyboard, and we read that back to drive the chatpad's Caps light.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::mem::size_of;
use std::os::fd::AsRawFd;

use x360truewireless_common::keycode::KeyCode;

const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const EV_LED: u16 = 0x11;
const SYN_REPORT: u16 = 0;
const LED_CAPSL: u16 = 0x01;
const BUS_VIRTUAL: u16 = 0x06;

// ioctl numbers from <linux/uinput.h>.
const UI_DEV_CREATE: libc::c_ulong = 0x5501;
const UI_DEV_DESTROY: libc::c_ulong = 0x5502;
const UI_DEV_SETUP: libc::c_ulong = 0x405c_5503;
const UI_SET_EVBIT: libc::c_ulong = 0x4004_5564;
const UI_SET_KEYBIT: libc::c_ulong = 0x4004_5565;
const UI_SET_LEDBIT: libc::c_ulong = 0x4004_5569;

#[repr(C)]
struct InputId {
    bustype: u16,
    vendor: u16,
    product: u16,
    version: u16,
}

#[repr(C)]
struct UinputSetup {
    id: InputId,
    name: [u8; 80],
    ff_effects_max: u32,
}

const _: () = assert!(size_of::<UinputSetup>() == 92);

#[repr(C)]
#[derive(Clone, Copy)]
struct InputEvent {
    time: libc::timeval,
    kind: u16,
    code: u16,
    value: i32,
}

pub struct Keyboard {
    file: File,
}

/// Reads Caps Lock LED changes the compositor writes to the keyboard.
pub struct LedReader {
    file: File,
}

impl Keyboard {
    pub fn new() -> io::Result<(Self, LedReader)> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/uinput")?;
        let fd = file.as_raw_fd();
        let ioctl = |request: libc::c_ulong, arg: libc::c_ulong| -> io::Result<()> {
            // SAFETY: uinput ioctls taking an integer argument, on a fd we own.
            if unsafe { libc::ioctl(fd, request as _, arg) } < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        };
        ioctl(UI_SET_EVBIT, EV_KEY as _)?;
        // Every regular key: which ones we press depends on the user's layout.
        for code in 1..=KeyCode::KEY_MICMUTE.0 {
            ioctl(UI_SET_KEYBIT, code as _)?;
        }
        ioctl(UI_SET_EVBIT, EV_LED as _)?;
        ioctl(UI_SET_LEDBIT, LED_CAPSL as _)?;

        let mut setup = UinputSetup {
            id: InputId {
                bustype: BUS_VIRTUAL,
                vendor: 0x045e,
                product: 0x0719,
                version: 1,
            },
            name: [0; 80],
            ff_effects_max: 0,
        };
        let name = b"X360 Chatpad";
        setup.name[..name.len()].copy_from_slice(name);
        // SAFETY: UI_DEV_SETUP reads a `struct uinput_setup`, which UinputSetup mirrors.
        if unsafe { libc::ioctl(fd, UI_DEV_SETUP as _, &setup) } < 0 {
            return Err(io::Error::last_os_error());
        }
        ioctl(UI_DEV_CREATE, 0)?;

        let reader = LedReader {
            file: file.try_clone()?,
        };
        Ok((Keyboard { file }, reader))
    }

    pub fn send(&mut self, events: &[(KeyCode, bool)]) -> io::Result<()> {
        // One SYN per key event, so compositors see presses in order and never merged.
        for &(key, down) in events {
            self.write_event(EV_KEY, key.0, down as i32)?;
            self.write_event(EV_SYN, SYN_REPORT, 0)?;
        }
        Ok(())
    }

    fn write_event(&mut self, kind: u16, code: u16, value: i32) -> io::Result<()> {
        let event = InputEvent {
            time: libc::timeval {
                tv_sec: 0,
                tv_usec: 0,
            },
            kind,
            code,
            value,
        };
        // SAFETY: InputEvent is a plain #[repr(C)] struct matching `struct input_event`.
        let bytes = unsafe {
            std::slice::from_raw_parts(
                &event as *const InputEvent as *const u8,
                size_of::<InputEvent>(),
            )
        };
        self.file.write_all(bytes)
    }
}

impl Drop for Keyboard {
    fn drop(&mut self) {
        // SAFETY: plain ioctl on our own fd.
        unsafe { libc::ioctl(self.file.as_raw_fd(), UI_DEV_DESTROY as _) };
    }
}

impl LedReader {
    /// Block until the Caps Lock LED changes; returns its new state.
    pub fn next_caps_lock(&mut self) -> io::Result<bool> {
        let mut buf = [0u8; size_of::<InputEvent>()];
        loop {
            self.file.read_exact(&mut buf)?;
            // SAFETY: buf holds exactly one `struct input_event` as written by the kernel.
            let event: InputEvent =
                unsafe { std::ptr::read_unaligned(buf.as_ptr() as *const InputEvent) };
            if event.kind == EV_LED && event.code == LED_CAPSL {
                return Ok(event.value != 0);
            }
        }
    }
}
