//! The daemon's D-Bus service (names in `x360truewireless_common::dbus`), used by the
//! tray/settings app. On Linux it's on the session bus; Windows has no bus, so there each
//! client connects to a Unix socket and gets a peer-to-peer connection with the same object.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, Weak};

#[cfg(target_os = "linux")]
use x360truewireless_common::dbus::BUS_NAME;
use x360truewireless_common::dbus::{INTERFACE, OBJECT_PATH};
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

/// The connections clients reach the service through, for signals.
#[derive(Clone, Default)]
pub struct Ipc {
    conns: Arc<Mutex<Vec<zbus::blocking::Connection>>>,
}

impl Ipc {
    pub fn emit_key(&self, slot: u8, key: ChatpadKey) {
        // A connection whose client has gone fails to send; drop it.
        self.conns.lock().unwrap().retain(|conn| {
            conn.emit_signal(
                None::<&str>,
                OBJECT_PATH,
                INTERFACE,
                "ChatpadKey",
                &(slot, key.name()),
            )
            .is_ok()
        });
    }
}

/// Start the service. Without a session bus (or if another daemon owns the name) we carry on
/// without it; only the settings app needs it.
#[cfg(target_os = "linux")]
pub fn serve(shared: Weak<Shared>) -> Ipc {
    let ipc = Ipc::default();
    let conn = zbus::blocking::connection::Builder::session()
        .and_then(|b| b.name(BUS_NAME))
        .and_then(|b| b.serve_at(OBJECT_PATH, Service { shared }))
        .and_then(|b| b.build());
    match conn {
        Ok(conn) => ipc.conns.lock().unwrap().push(conn),
        Err(e) => {
            eprintln!("D-Bus service unavailable ({e}); the settings app won't be able to connect")
        }
    }
    ipc
}

/// Start the service: listen on the socket and serve each client that connects.
#[cfg(windows)]
pub fn serve(shared: Weak<Shared>) -> Ipc {
    let ipc = Ipc::default();
    let Some(path) = x360truewireless_common::dbus::socket_path() else {
        eprintln!("no local app data folder; the settings app won't be able to connect");
        return ipc;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // A socket file left over from a previous run would make the bind fail.
    let _ = std::fs::remove_file(&path);
    let listener = match uds_windows::UnixListener::bind(&path) {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!(
                "can't listen on {} ({e}); the settings app won't be able to connect",
                path.display()
            );
            return ipc;
        }
    };
    let conns = ipc.conns.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let conn = stream.map_err(zbus::Error::from).and_then(|stream| {
                zbus::blocking::connection::Builder::async_io_unix_stream(stream)
                    .server(zbus::Guid::generate())?
                    .p2p()
                    .serve_at(
                        OBJECT_PATH,
                        Service {
                            shared: shared.clone(),
                        },
                    )?
                    .build()
            });
            match conn {
                Ok(conn) => conns.lock().unwrap().push(conn),
                Err(e) => eprintln!("settings app connection failed: {e}"),
            }
        }
    });
    ipc
}
