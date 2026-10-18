//! The system tray icon (StatusNotifierItem): driver status plus a menu to open settings.

use std::collections::HashMap;
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use ksni::blocking::TrayMethods;
use ksni::menu::{MenuItem, StandardItem};
use x360truewireless_common::Battery;

use crate::client::{Client, Status};

struct Tray {
    status: Status,
    /// Our icon, drawn in the panel's text colour.
    icons: Vec<ksni::Icon>,
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

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "x360truewireless".into()
    }

    fn title(&self) -> String {
        "X360TrueWireless".into()
    }

    fn icon_name(&self) -> String {
        // Empty: hosts then use `icon_pixmap`, which always works.
        String::new()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.icons.clone()
    }

    fn status(&self) -> ksni::Status {
        // Passive tucks the icon into the overflow area on desktops that support it.
        match &self.status.running {
            Some(s) if s.connected.iter().any(|c| *c) => ksni::Status::Active,
            _ => ksni::Status::Passive,
        }
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "X360TrueWireless".into(),
            description: self.tool_tip_text(),
            ..Default::default()
        }
    }

    fn activate(&mut self, x: i32, y: i32) {
        open_settings(Some((x, y)));
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let mut items: Vec<MenuItem<Self>> = vec![label(self.summary())];
        if let Some(s) = &self.status.running {
            for (i, connected) in s.connected.iter().enumerate() {
                if *connected {
                    items.push(label(format!("    Player {}", i + 1)));
                }
            }
            items.push(label(format!("Keyboard layout: {}", s.layout)));
        }
        items.push(MenuItem::Separator);
        items.push(
            StandardItem {
                label: "Settings…".into(),
                icon_name: "configure".into(),
                activate: Box::new(|_| open_settings(None)),
                ..Default::default()
            }
            .into(),
        );
        if self.status.running.is_some() {
            items.push(
                StandardItem {
                    label: "Reload settings and keyboard layout".into(),
                    icon_name: "view-refresh".into(),
                    activate: Box::new(|_| {
                        if let Err(e) = Client::new().reload() {
                            eprintln!("reload: {e}");
                        }
                    }),
                    ..Default::default()
                }
                .into(),
            );
        }
        items.push(
            StandardItem {
                label: if self.status.running.is_some() {
                    "Restart driver"
                } else {
                    "Start driver"
                }
                .into(),
                icon_name: "system-reboot".into(),
                activate: Box::new(|_| restart_driver()),
                ..Default::default()
            }
            .into(),
        );
        items.push(MenuItem::Separator);
        items.push(
            StandardItem {
                label: "Hide tray icon".into(),
                icon_name: "window-close".into(),
                activate: Box::new(|_| std::process::exit(0)),
                ..Default::default()
            }
            .into(),
        );
        items
    }
}

fn label(text: String) -> MenuItem<Tray> {
    StandardItem {
        label: text,
        enabled: false,
        ..Default::default()
    }
    .into()
}

/// The open settings popup, if any, and when the last one closed.
static POPUP: Mutex<(Option<u32>, Option<Instant>)> = Mutex::new((None, None));

/// Toggle the settings popup. Clicking the tray icon while the popup is open closes it.
/// (Clicking the icon also takes focus from the popup, which then closes itself; a click
/// right after that is the same click, so it mustn't reopen the popup.)
pub fn open_settings(at: Option<(i32, i32)>) {
    let mut popup = POPUP.lock().unwrap();
    if let Some(pid) = popup.0 {
        let _ = Command::new("kill").arg(pid.to_string()).status();
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

/// Show a desktop notification (org.freedesktop.Notifications) for a low controller.
fn notify_low_battery(slot: usize) {
    let Ok(conn) = zbus::blocking::Connection::session() else {
        return;
    };
    let _ = conn.call_method(
        Some("org.freedesktop.Notifications"),
        "/org/freedesktop/Notifications",
        Some("org.freedesktop.Notifications"),
        "Notify",
        &(
            "X360TrueWireless",
            0u32,
            "input-gaming",
            format!("Player {} battery is low", slot + 1),
            "Charge or replace the controller's batteries.",
            Vec::<&str>::new(),
            HashMap::<&str, zbus::zvariant::Value>::new(),
            -1i32,
        ),
    );
}

pub fn restart_driver() {
    let _ = Command::new("systemctl")
        .args(["--user", "restart", "x360truewireless.service"])
        .status();
}

/// Run the tray until killed. Returns an error if the desktop has no tray (e.g. GNOME without
/// the AppIndicator extension).
pub fn run() -> anyhow::Result<()> {
    let text = crate::theme::Palette::detect().text;
    let icons = crate::icon::tray_pixmaps([text.r(), text.g(), text.b()]);
    let handle = Tray {
        status: Status::default(),
        icons,
    }
    .spawn()
    .map_err(|e| anyhow::anyhow!("no system tray available ({e})"))?;
    let notified = Mutex::new([false; 4]);
    crate::client::watch_status(Duration::from_secs(2), move |status| {
        if let Some(s) = &status.running {
            let mut notified = notified.lock().unwrap();
            for slot in newly_low(&mut *notified, &s.connected, &s.battery) {
                notify_low_battery(slot);
            }
        }
        let status = status.clone();
        handle.update(move |tray: &mut Tray| tray.status = status);
    });
    loop {
        std::thread::park();
    }
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
            icons: Vec::new(),
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
