//! The daemon's D-Bus service on the session bus (names in `x360truewireless_common::dbus`), used
//! by the tray/settings app.

use std::sync::Weak;
use std::sync::atomic::Ordering;

use x360truewireless_common::dbus::{BUS_NAME, INTERFACE, OBJECT_PATH};
use x360truewireless_proto::ChatpadKey;

use crate::Shared;

struct Service {
    shared: Weak<Shared>,
}

#[zbus::interface(name = "io.github.NewAbbreviations950.X360TrueWireless1")]
impl Service {
    /// Re-read config.toml and re-detect the keyboard layout. Fails with the config problems,
    /// if any (the valid parts are applied anyway).
    fn reload(&self) -> zbus::fdo::Result<()> {
        let shared = self
            .shared
            .upgrade()
            .ok_or(zbus::fdo::Error::Failed("shutting down".into()))?;
        let problems = shared.reload();
        if problems.is_empty() {
            Ok(())
        } else {
            Err(zbus::fdo::Error::Failed(problems.join("\n")))
        }
    }

    /// The active keyboard layout and which of the four slots have a controller.
    fn status(&self) -> zbus::fdo::Result<(String, Vec<bool>)> {
        let shared = self
            .shared
            .upgrade()
            .ok_or(zbus::fdo::Error::Failed("shutting down".into()))?;
        let layout = shared.keymap.read().unwrap().layout().name.to_string();
        let connected = shared
            .connected
            .iter()
            .map(|c| c.load(Ordering::SeqCst))
            .collect();
        Ok((layout, connected))
    }

    /// Battery bars (1 empty to 4 full) for each of the four slots; 0 when unknown or not connected.
    fn battery(&self) -> zbus::fdo::Result<Vec<u8>> {
        let shared = self
            .shared
            .upgrade()
            .ok_or(zbus::fdo::Error::Failed("shutting down".into()))?;
        let bars = shared
            .connected
            .iter()
            .zip(&shared.battery)
            .map(|(connected, bars)| {
                if connected.load(Ordering::SeqCst) {
                    bars.load(Ordering::SeqCst)
                } else {
                    0
                }
            })
            .collect();
        Ok(bars)
    }

    /// What each of the four slots has connected, as `SlotDevice` codes.
    fn devices(&self) -> zbus::fdo::Result<Vec<u8>> {
        let shared = self
            .shared
            .upgrade()
            .ok_or(zbus::fdo::Error::Failed("shutting down".into()))?;
        Ok(shared
            .devices
            .iter()
            .map(|d| d.load(Ordering::SeqCst))
            .collect())
    }
}

/// Start the service. Without a session bus (or if another daemon owns the name) we carry on
/// without it; only the settings app needs it.
pub fn serve(shared: Weak<Shared>) -> Option<zbus::blocking::Connection> {
    let conn = zbus::blocking::connection::Builder::session()
        .and_then(|b| b.name(BUS_NAME))
        .and_then(|b| b.serve_at(OBJECT_PATH, Service { shared }))
        .and_then(|b| b.build());
    match conn {
        Ok(conn) => Some(conn),
        Err(e) => {
            eprintln!("D-Bus service unavailable ({e}); the settings app won't be able to connect");
            None
        }
    }
}

pub fn emit_key(conn: &zbus::blocking::Connection, slot: u8, key: ChatpadKey) {
    let _ = conn.emit_signal(
        None::<&str>,
        OBJECT_PATH,
        INTERFACE,
        "ChatpadKey",
        &(slot, key.name()),
    );
}
