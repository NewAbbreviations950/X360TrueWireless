//! The tray icon on Linux: a StatusNotifierItem (KDE, GNOME with AppIndicator, and others).

use std::collections::HashMap;
use std::process::Command;
use std::sync::Mutex;
use std::time::Duration;

use ksni::blocking::TrayMethods;
use ksni::menu::{MenuItem, StandardItem};

use super::{Tray, newly_low, open_settings};
use crate::client::Client;

/// The tray, with our icon drawn in the panel's text colour.
struct Sni {
    tray: Tray,
    icons: Vec<ksni::Icon>,
}

impl ksni::Tray for Sni {
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
        match &self.tray.status.running {
            Some(s) if s.connected.iter().any(|c| *c) => ksni::Status::Active,
            _ => ksni::Status::Passive,
        }
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "X360TrueWireless".into(),
            description: self.tray.tool_tip_text(),
            ..Default::default()
        }
    }

    fn activate(&mut self, x: i32, y: i32) {
        open_settings(Some((x, y)));
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let mut items: Vec<MenuItem<Self>> = vec![label(self.tray.summary())];
        if let Some(s) = &self.tray.status.running {
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
        if self.tray.status.running.is_some() {
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
                label: if self.tray.status.running.is_some() {
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

fn label(text: String) -> MenuItem<Sni> {
    StandardItem {
        label: text,
        enabled: false,
        ..Default::default()
    }
    .into()
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
    let icons = crate::icon::tray_pixmaps([text.r(), text.g(), text.b()])
        .into_iter()
        .map(sni_icon)
        .collect();
    let handle = Sni {
        tray: Tray::default(),
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
        handle.update(move |sni: &mut Sni| sni.tray.status = status);
    });
    loop {
        std::thread::park();
    }
}

/// StatusNotifierItem wants straight ARGB, big-endian.
fn sni_icon(icon: crate::icon::Rendered) -> ksni::Icon {
    ksni::Icon {
        width: icon.size as i32,
        height: icon.size as i32,
        data: icon
            .rgba
            .chunks(4)
            .flat_map(|p| [p[3], p[0], p[1], p[2]])
            .collect(),
    }
}
