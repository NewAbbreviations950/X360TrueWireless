//! The system tray icon: driver status plus a menu to open settings. The icon itself is
//! a StatusNotifierItem on Linux and a notification-area icon on Windows.

use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use x360truewireless_common::Battery;

use crate::client::Status;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{restart_driver, run};
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{restart_driver, run, setup_receiver};

/// What the tray shows.
#[derive(Default)]
struct Tray {
    status: Status,
}

impl Tray {
    fn summary(&self) -> String {
        match &self.status.running {
            None => "Driver not running".into(),
            Some(s) => match s.connected.iter().filter(|c| **c).count() {
                0 => "No controllers connected".into(),
                1 => "1 controller connected".into(),
                n => format!("{n} controllers connected"),
            },
        }
    }

    /// The summary, then a line per connected player with its battery level when known.
    fn tool_tip_text(&self) -> String {
        let mut lines = vec![self.summary()];
        if let Some(s) = &self.status.running {
            for (slot, connected) in s.connected.iter().enumerate() {
                if !*connected {
                    continue;
                }
                lines.push(match s.battery.get(slot).copied().flatten() {
                    Some(level) => format!("Player {}: battery {}", slot + 1, level.name()),
                    None => format!("Player {}", slot + 1),
                });
            }
        }
        lines.join("\n")
    }
}

/// The open settings popup, if any, and when the last one closed.
static POPUP: Mutex<(Option<u32>, Option<Instant>)> = Mutex::new((None, None));

/// Toggle the settings popup. Clicking the tray icon while the popup is open closes it.
/// (Clicking the icon also takes focus from the popup, which then closes itself; a click
/// right after that is the same click, so it mustn't reopen the popup.)
pub fn open_settings(at: Option<(i32, i32)>) {
    let mut popup = POPUP.lock().unwrap();
    if let Some(pid) = popup.0 {
        kill(pid);
        return;
    }
    if popup
        .1
        .is_some_and(|t| t.elapsed() < Duration::from_millis(400))
    {
        return;
    }
    let exe = std::env::current_exe().unwrap_or_else(|_| "x360truewireless-tray".into());
    let mut command = Command::new(exe);
    command.arg("--settings");
    if let Some((x, y)) = at {
        command.arg(format!("{x},{y}"));
    }
    match command.spawn() {
        Ok(mut child) => {
            popup.0 = Some(child.id());
            std::thread::spawn(move || {
                // Reap it, and note when it closed.
                let _ = child.wait();
                *POPUP.lock().unwrap() = (None, Some(Instant::now()));
            });
        }
        Err(e) => eprintln!("opening settings: {e}"),
    }
}

#[cfg(target_os = "linux")]
fn kill(pid: u32) {
    let _ = Command::new("kill").arg(pid.to_string()).status();
}

#[cfg(windows)]
fn kill(pid: u32) {
    use std::os::windows::process::CommandExt;
    // CREATE_NO_WINDOW: no console flashing up.
    let _ = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .creation_flags(0x0800_0000)
        .status();
}

/// The slots whose controller has just dropped to a low battery. `notified` remembers which
/// slots were already reported, so each low spell notifies once; a slot is re-armed when its
/// controller disconnects or its level climbs back above low.
fn newly_low(notified: &mut [bool], connected: &[bool], battery: &[Option<Battery>]) -> Vec<usize> {
    let mut fresh = Vec::new();
    for (slot, flag) in notified.iter_mut().enumerate() {
        let low = connected.get(slot).copied().unwrap_or(false)
            && battery
                .get(slot)
                .copied()
                .flatten()
                .is_some_and(Battery::is_low);
        if low && !*flag {
            fresh.push(slot);
        }
        *flag = low;
    }
    fresh
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn low_battery_notifies_once_per_spell() {
        let mut notified = [false; 4];
        let connected = [true, false, false, false];
        let low = [Some(Battery::Low), None, None, None];
        let full = [Some(Battery::Full), None, None, None];

        assert_eq!(newly_low(&mut notified, &connected, &low), vec![0]);
        assert!(newly_low(&mut notified, &connected, &low).is_empty());
        // Recovering re-arms the slot, so a later drop notifies again.
        assert!(newly_low(&mut notified, &connected, &full).is_empty());
        assert_eq!(newly_low(&mut notified, &connected, &low), vec![0]);
    }

    fn tray(connected: [bool; 4], battery: [Option<Battery>; 4]) -> Tray {
        Tray {
            status: Status {
                running: Some(crate::client::DaemonStatus {
                    layout: "no".into(),
                    connected: connected.into(),
                    battery: battery.into(),
                    devices: Vec::new(),
                }),
            },
        }
    }

    #[test]
    fn tool_tip_lists_battery_per_connected_player() {
        let t = tray(
            [true, false, true, false],
            [Some(Battery::Full), Some(Battery::Low), None, None],
        );
        // Slot 2's stale level is skipped; slot 3 has no level yet.
        assert_eq!(
            t.tool_tip_text(),
            "2 controllers connected\nPlayer 1: battery full\nPlayer 3"
        );
    }

    #[test]
    fn tool_tip_without_driver_or_controllers() {
        let mut t = tray([false; 4], [None; 4]);
        assert_eq!(t.tool_tip_text(), "No controllers connected");
        t.status.running = None;
        assert_eq!(t.tool_tip_text(), "Driver not running");
    }

    #[test]
    fn disconnect_rearms_and_unconnected_slots_stay_quiet() {
        let mut notified = [false; 4];
        let low = [Some(Battery::Empty), Some(Battery::Empty), None, None];
        // Slot 2 has a stale level but no controller: no notification.
        assert_eq!(
            newly_low(&mut notified, &[true, false, false, false], &low),
            vec![0]
        );
        assert!(newly_low(&mut notified, &[false, false, false, false], &low).is_empty());
        assert_eq!(
            newly_low(&mut notified, &[true, false, false, false], &low),
            vec![0]
        );
    }
}
