//! Windows: ViGEmBus pads, vJoy wheels, `SendInput` typing and a receiver on WinUSB.

pub mod actions;
mod devices;
mod keyboard;
pub mod layout;
pub mod receiver;
mod vjoy;

use std::fs::OpenOptions;
use std::os::windows::io::AsRawHandle;
use std::path::PathBuf;

use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
use windows_sys::Win32::System::Console::{STD_ERROR_HANDLE, SetStdHandle};
use windows_sys::Win32::System::Threading::CreateMutexW;

pub use devices::{VirtualPad, Wheel};
pub use keyboard::Keyboard;

/// Log files bigger than this start over.
const LOG_LIMIT: u64 = 1 << 20;

fn data_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("LOCALAPPDATA").filter(|v| !v.is_empty())?;
    Some(PathBuf::from(dir).join("X360TrueWireless"))
}

/// Start-up: run only once per user, and log to `%LOCALAPPDATA%\X360TrueWireless\daemon.log`
/// (the driver has no console window). Returns false if the driver is already running.
pub fn init() -> bool {
    let name: Vec<u16> = r"Local\X360TrueWireless.daemon"
        .encode_utf16()
        .chain([0])
        .collect();
    // SAFETY: a null-terminated name; the mutex is kept for the life of the process.
    unsafe {
        CreateMutexW(std::ptr::null(), 0, name.as_ptr());
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return false;
        }
    }
    if let Some(dir) = data_dir() {
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("daemon.log");
        let keep = std::fs::metadata(&path).is_ok_and(|m| m.len() < LOG_LIMIT);
        let file = OpenOptions::new()
            .create(true)
            .append(keep)
            .write(true)
            .truncate(!keep)
            .open(&path);
        if let Ok(file) = file {
            // SAFETY: the handle stays open for the life of the process (leaked below).
            unsafe { SetStdHandle(STD_ERROR_HANDLE, file.as_raw_handle()) };
            std::mem::forget(file);
        }
    }
    true
}
