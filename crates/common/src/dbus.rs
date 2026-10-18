//! D-Bus names shared by the daemon and the tray/settings app (session bus).
//!
//! Methods: `Reload()` re-reads config.toml and re-detects the keyboard layout;
//! `Status() -> (layout: s, connected: ab)` gives the active layout and which of the four
//! controller slots are connected.
//! `Battery() -> ay` gives each slot's battery level as bars (1 empty to 4 full), or 0 when
//! unknown or not connected.
//! Signal: `ChatpadKey(slot: y, key: s)` when a chatpad key is pressed (`ChatpadKey` name).

pub const BUS_NAME: &str = "io.github.NewAbbreviations950.X360TrueWireless";
pub const OBJECT_PATH: &str = "/io/github/NewAbbreviations950/X360TrueWireless";
pub const INTERFACE: &str = "io.github.NewAbbreviations950.X360TrueWireless1";
