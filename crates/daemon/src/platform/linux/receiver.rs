//! Taking the receiver over from `xpad`, and handing it back.

use std::path::Path;
use std::thread;
use std::time::Duration;

use nusb::{DeviceInfo, Interface, MaybeFuture};

use crate::SLOTS;

/// Where the receiver is, for the log.
pub fn describe(info: &DeviceInfo) -> String {
    info.sysfs_path().display().to_string()
}

/// Claim a controller interface, detaching `xpad` from it.
pub fn claim(device: &nusb::Device, iface: u8) -> Result<Interface, nusb::Error> {
    device.detach_and_claim_interface(iface).wait()
}

/// Hand every controller interface back to xpad. xpad's probe occasionally fails right after
/// a release, so check each interface actually got a driver and retry a few times.
pub fn release(device: &nusb::Device, info: &DeviceInfo) {
    let sysfs = info.sysfs_path();
    let name = sysfs
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    for slot in 0..SLOTS {
        let iface = slot * 2;
        let driver_link = sysfs.join(format!("{name}:1.{iface}/driver"));
        let mut last_err = None;
        for attempt in 0..5 {
            if has_driver(&driver_link) {
                break;
            }
            if attempt > 0 {
                thread::sleep(Duration::from_millis(200));
            }
            if let Err(e) = device.attach_kernel_driver(iface) {
                last_err = Some(e);
            }
        }
        if !has_driver(&driver_link) && sysfs.exists() {
            match last_err {
                Some(e) => eprintln!(
                    "warning: could not give interface {iface} back to xpad after 5 attempts: {e}"
                ),
                None => eprintln!(
                    "warning: could not give interface {iface} back to xpad after 5 attempts"
                ),
            }
        }
    }
}

/// True only when the interface is bound to xpad. Any driver would do for a plain existence
/// check, but while we still hold the interface its link points at usbfs, which would make
/// the handback think it had already succeeded.
fn has_driver(link: &Path) -> bool {
    std::fs::read_link(link).is_ok_and(|target| target.file_name() == Some("xpad".as_ref()))
}
