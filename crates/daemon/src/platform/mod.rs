//! What differs between operating systems: the virtual devices, typing, the receiver
//! handover and desktop actions. Each platform module offers the same names.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::*;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;
