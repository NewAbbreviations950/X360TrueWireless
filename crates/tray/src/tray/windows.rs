//! The tray icon on Windows: a notification-area icon with a context menu. Left click opens
//! the settings popup, like on Linux.

use std::process::Command;
use std::sync::{Mutex, mpsc};
use std::time::Duration;

use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::System::Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject};
use windows_sys::Win32::UI::Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MB_ICONERROR, MB_ICONINFORMATION, MB_OK, MSG, MessageBoxW, PM_REMOVE,
    PeekMessageW, SW_HIDE, TranslateMessage,
};

use super::{Tray, newly_low, open_settings};
use crate::client::{Client, Status};

const DAEMON_EXE: &str = "x360truewireless-daemon.exe";
/// CREATE_NO_WINDOW: helpers run without a console flashing up.
const NO_WINDOW: u32 = 0x0800_0000;

fn menu(tray: &Tray) -> Menu {
    let menu = Menu::new();
    let label = |text: String| MenuItem::new(text, false, None);
    let _ = menu.append(&label(tray.summary()));
    if let Some(s) = &tray.status.running {
        for (i, connected) in s.connected.iter().enumerate() {
            if *connected {
                let _ = menu.append(&label(format!("    Player {}", i + 1)));
            }
        }
        let _ = menu.append(&label(format!("Keyboard layout: {}", s.layout)));
    }
    let _ = menu.append(&PredefinedMenuItem::separator());
    let _ = menu.append(&MenuItem::with_id("settings", "Settings…", true, None));
    if tray.status.running.is_some() {
        let _ = menu.append(&MenuItem::with_id(
            "reload",
            "Reload settings and keyboard layout",
            true,
            None,
        ));
    }
    let restart = if tray.status.running.is_some() {
        "Restart driver"
    } else {
        "Start driver"
    };
    let _ = menu.append(&MenuItem::with_id("restart", restart, true, None));
    let _ = menu.append(&MenuItem::with_id(
        "receiver",
        "Set up receiver…",
        true,
        None,
    ));
    let _ = menu.append(&PredefinedMenuItem::separator());
    let _ = menu.append(&MenuItem::with_id("quit", "Hide tray icon", true, None));
    menu
}

/// The icon in white on a dark taskbar, dark on a light one.
fn icon() -> Option<Icon> {
    let rgb = if crate::theme::light_taskbar() {
        [35, 38, 41]
    } else {
        [255, 255, 255]
    };
    // Windows draws tray icons at 16 px at 100% scaling; the largest is scaled down cleanly.
    let icon = crate::icon::tray_pixmaps(rgb).pop()?;
    Icon::from_rgba(icon.rgba, icon.size, icon.size).ok()
}

fn notify_low_battery(slot: usize) {
    let _ =
        tauri_winrt_notification::Toast::new(tauri_winrt_notification::Toast::POWERSHELL_APP_ID)
            .title(&format!("Player {} battery is low", slot + 1))
            .text1("Charge or replace the controller's batteries.")
            .show();
}

/// Stop the driver if it's running, and start it again from next to this program.
pub fn restart_driver() {
    use std::os::windows::process::CommandExt;
    let _ = Command::new("taskkill")
        .args(["/IM", DAEMON_EXE, "/F"])
        .creation_flags(NO_WINDOW)
        .status();
    // Give the receiver a moment to be released.
    std::thread::sleep(Duration::from_millis(500));
    let exe = std::env::current_exe()
        .map(|e| e.with_file_name(DAEMON_EXE))
        .unwrap_or_else(|_| DAEMON_EXE.into());
    if let Err(e) = Command::new(&exe).spawn() {
        eprintln!("starting {}: {e}", exe.display());
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

fn message(text: &str, flags: u32) {
    let (text, title) = (wide(text), wide("X360TrueWireless"));
    // SAFETY: null-terminated strings that outlive the call.
    unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), flags) };
}

/// Put WinUSB on the receiver (after it moved to another USB port, say): the installer's
/// script, run as administrator, then the driver restarted to pick it up.
pub fn setup_receiver() {
    let script = std::env::current_exe()
        .map(|e| e.with_file_name("receiver-driver.ps1"))
        .unwrap_or_default();
    let (verb, file) = (wide("runas"), wide("powershell.exe"));
    let params = wide(&format!(
        "-NoProfile -ExecutionPolicy Bypass -File \"{}\"",
        script.display()
    ));
    // SAFETY: a zeroed SHELLEXECUTEINFOW is valid; the strings outlive the call, and the
    // process handle is closed below.
    let code = unsafe {
        let mut info: SHELLEXECUTEINFOW = std::mem::zeroed();
        info.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_NOCLOSEPROCESS;
        info.lpVerb = verb.as_ptr();
        info.lpFile = file.as_ptr();
        info.lpParameters = params.as_ptr();
        info.nShow = SW_HIDE;
        // Fails if the user declines the administrator prompt.
        if ShellExecuteExW(&mut info) == 0 || info.hProcess.is_null() {
            return;
        }
        WaitForSingleObject(info.hProcess, INFINITE);
        let mut code = 1;
        GetExitCodeProcess(info.hProcess, &mut code);
        CloseHandle(info.hProcess);
        code
    };
    match code {
        0 => {
            restart_driver();
            message("The receiver is set up.", MB_OK | MB_ICONINFORMATION);
        }
        2 => message(
            "No receiver is plugged in. Plug it in and try again.",
            MB_OK | MB_ICONINFORMATION,
        ),
        _ => message(
            &format!("Setting up the receiver failed (code {code})."),
            MB_OK | MB_ICONERROR,
        ),
    }
}

/// Run the tray until quit. The icon belongs to this thread, which also runs the window
/// messages it needs; status updates arrive over a channel.
pub fn run() -> anyhow::Result<()> {
    let (tx, rx) = mpsc::channel::<Status>();
    let notified = Mutex::new([false; 4]);
    crate::client::watch_status(Duration::from_secs(2), move |status| {
        if let Some(s) = &status.running {
            let mut notified = notified.lock().unwrap();
            for slot in newly_low(&mut *notified, &s.connected, &s.battery) {
                notify_low_battery(slot);
            }
        }
        let _ = tx.send(status.clone());
    });

    let mut tray = Tray::default();
    let mut builder = TrayIconBuilder::new()
        .with_tooltip(tray.tool_tip_text())
        .with_menu(Box::new(menu(&tray)))
        .with_menu_on_left_click(false);
    if let Some(icon) = icon() {
        builder = builder.with_icon(icon);
    }
    let icon = builder
        .build()
        .map_err(|e| anyhow::anyhow!("no notification area available ({e})"))?;

    loop {
        // SAFETY: a standard message pump on the thread that owns the icon's window.
        unsafe {
            let mut msg: MSG = std::mem::zeroed();
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        while let Ok(status) = rx.try_recv() {
            tray.status = status;
            let _ = icon.set_tooltip(Some(tray.tool_tip_text()));
            icon.set_menu(Some(Box::new(menu(&tray))));
        }
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            match event.id.0.as_str() {
                "settings" => open_settings(None),
                "reload" => {
                    if let Err(e) = Client::new().reload() {
                        eprintln!("reload: {e}");
                    }
                }
                "restart" => restart_driver(),
                "receiver" => {
                    std::thread::spawn(setup_receiver);
                }
                "quit" => std::process::exit(0),
                _ => {}
            }
        }
        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                position,
                ..
            } = event
            {
                open_settings(Some((position.x as i32, position.y as i32)));
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
