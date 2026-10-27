//! Desktop actions triggered from the chatpad.

use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

/// Open Steam's friends & chat window, through Steam's URL handler.
pub fn open_steam_chat() {
    let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let (verb, url) = (wide("open"), wide("steam://open/friends"));
    // SAFETY: null-terminated strings that outlive the call.
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            url.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    // Values up to 32 are errors.
    if result as usize <= 32 {
        eprintln!("opening Steam chat failed: is Steam installed?");
    }
}
