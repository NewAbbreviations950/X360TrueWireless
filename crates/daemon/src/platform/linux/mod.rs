//! Linux: uinput devices, xkbcommon layouts, and handing the receiver back to `xpad`.

pub mod actions;
mod devices;
mod ffb_raw;
mod keyboard;
pub mod layout;
pub mod receiver;

pub use devices::{VirtualPad, Wheel};
pub use keyboard::Keyboard;

/// Start-up; systemd keeps the driver to one instance and collects its log.
pub fn init() -> bool {
    true
}
