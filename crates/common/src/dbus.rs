//! D-Bus names shared by the daemon and the tray/settings app (session bus).
//!
//! Methods: `Reload()` re-reads config.toml and re-detects the keyboard layout;
//! `Status() -> (layout: s, connected: ab)` gives the active layout and which of the four
//! controller slots are connected.
//! `Battery() -> ay` gives each slot's battery level as bars (1 empty to 4 full), or 0 when
//! unknown or not connected.
//! `Devices() -> ay` gives what each slot has connected, as [`SlotDevice`] codes.
//! Signal: `ChatpadKey(slot: y, key: s)` when a chatpad key is pressed (`ChatpadKey` name).

pub const BUS_NAME: &str = "io.github.NewAbbreviations950.X360TrueWireless";
pub const OBJECT_PATH: &str = "/io/github/NewAbbreviations950/X360TrueWireless";
pub const INTERFACE: &str = "io.github.NewAbbreviations950.X360TrueWireless1";

/// What a controller slot has connected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotDevice {
    Empty,
    /// Connected, but it hasn't said yet whether it's a controller or a wheel.
    Connecting,
    Controller,
    /// A controller with a chatpad attached.
    Chatpad,
    Wheel,
}

impl SlotDevice {
    pub fn code(self) -> u8 {
        self as u8
    }

    /// Unknown codes (from a newer daemon) read as a plain controller.
    pub fn from_code(code: u8) -> Self {
        match code {
            0 => SlotDevice::Empty,
            1 => SlotDevice::Connecting,
            3 => SlotDevice::Chatpad,
            4 => SlotDevice::Wheel,
            _ => SlotDevice::Controller,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SlotDevice;

    #[test]
    fn slot_device_codes_round_trip() {
        use SlotDevice::*;
        for device in [Empty, Connecting, Controller, Chatpad, Wheel] {
            assert_eq!(SlotDevice::from_code(device.code()), device);
        }
    }
}
