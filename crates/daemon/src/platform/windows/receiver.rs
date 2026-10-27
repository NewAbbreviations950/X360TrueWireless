//! The receiver on Windows: the installer puts WinUSB on it, so claiming needs no handover and
//! there's no other driver to give it back to.

use nusb::{DeviceInfo, Interface, MaybeFuture};

/// Where the receiver is, for the log.
pub fn describe(info: &DeviceInfo) -> String {
    format!(
        "{} (port {:?})",
        info.instance_id().display(),
        info.port_chain()
    )
}

pub fn claim(device: &nusb::Device, iface: u8) -> Result<Interface, nusb::Error> {
    device.claim_interface(iface).wait()
}

pub fn release(_device: &nusb::Device, _info: &DeviceInfo) {}
