//! Pieces shared by the daemon and the tray/settings app: the config file, the chatpad
//! keymap and the D-Bus names.

pub mod config;
pub mod dbus;
pub mod keymap;

pub use x360truewireless_proto::{Battery, ChatpadKey};
