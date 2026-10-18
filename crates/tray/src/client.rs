//! Talking to x360truewireless-daemon over the session bus.

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use x360truewireless_common::dbus::{BUS_NAME, INTERFACE, OBJECT_PATH};
use x360truewireless_common::{Battery, ChatpadKey};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Status {
    /// None when the daemon isn't reachable.
    pub running: Option<DaemonStatus>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DaemonStatus {
    pub layout: String,
    pub connected: Vec<bool>,
    /// Per slot: the battery level, or None when unknown (or the slot is empty).
    pub battery: Vec<Option<Battery>>,
}

pub struct Client {
    conn: Option<zbus::blocking::Connection>,
}

impl Client {
    pub fn new() -> Self {
        let conn = zbus::blocking::Connection::session()
            .map_err(|e| eprintln!("no session bus: {e}"))
            .ok();
        Client { conn }
    }

    fn proxy(&self) -> Option<zbus::blocking::Proxy<'_>> {
        zbus::blocking::Proxy::new(self.conn.as_ref()?, BUS_NAME, OBJECT_PATH, INTERFACE).ok()
    }

    pub fn status(&self) -> Status {
        let running = self.proxy().and_then(|p| {
            let (layout, connected) = p.call::<_, _, (String, Vec<bool>)>("Status", &()).ok()?;
            // A daemon from before `Battery` existed has no such method: no bars then.
            let battery = p
                .call::<_, _, Vec<u8>>("Battery", &())
                .unwrap_or_default()
                .into_iter()
                .map(Battery::from_bars)
                .collect();
            Some(DaemonStatus {
                layout,
                connected,
                battery,
            })
        });
        Status { running }
    }

    /// Ask the daemon to re-read config.toml. Returns its complaints, if any.
    pub fn reload(&self) -> Result<(), String> {
        let proxy = self.proxy().ok_or("the driver isn't running")?;
        proxy.call::<_, _, ()>("Reload", &()).map_err(|e| match e {
            zbus::Error::MethodError(_, Some(msg), _) => msg,
            e => e.to_string(),
        })
    }
}

/// Poll the daemon's status every `interval` on a background thread, calling `on_change`
/// whenever it changes. Returns the latest status for synchronous reads.
pub fn watch_status(
    interval: Duration,
    on_change: impl Fn(&Status) + Send + 'static,
) -> Arc<Mutex<Status>> {
    let latest = Arc::new(Mutex::new(Status::default()));
    let shared = latest.clone();
    thread::spawn(move || {
        let client = Client::new();
        let mut first = true;
        loop {
            let status = client.status();
            let changed = *shared.lock().unwrap() != status;
            if changed || first {
                *shared.lock().unwrap() = status.clone();
                on_change(&status);
                first = false;
            }
            thread::sleep(interval);
        }
    });
    latest
}

/// Call `on_key` for every chatpad key pressed on any controller (from the daemon's
/// `ChatpadKey` signal). Re-subscribes if the daemon restarts.
pub fn watch_keys(on_key: impl Fn(u8, ChatpadKey) + Send + 'static) {
    thread::spawn(move || {
        loop {
            let client = Client::new();
            if let Some(proxy) = client.proxy()
                && let Ok(signals) = proxy.receive_signal("ChatpadKey")
            {
                for msg in signals {
                    if let Ok((slot, name)) = msg.body().deserialize::<(u8, String)>()
                        && let Some(key) = ChatpadKey::from_name(&name)
                    {
                        on_key(slot, key);
                    }
                }
            }
            thread::sleep(Duration::from_secs(2));
        }
    });
}
