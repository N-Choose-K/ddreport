//! USB bridge details for a block device, read from Linux sysfs.

use crate::model::UsbBridge;

/// Returns the USB bridge behind `device` (e.g. "/dev/sdb"), if any.
#[cfg(target_os = "linux")]
pub fn lookup(device: &str) -> Option<UsbBridge> {
    use std::fs;
    use std::path::Path;

    let name = Path::new(device).file_name()?.to_str()?;
    let mut dir = fs::canonicalize(format!("/sys/class/block/{name}/device")).ok()?;
    let read = |p: &Path, f: &str| {
        fs::read_to_string(p.join(f))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let mut transport = None;
    loop {
        if transport.is_none() && dir.join("bInterfaceClass").exists() {
            transport = fs::read_link(dir.join("driver"))
                .ok()
                .and_then(|d| d.file_name()?.to_str().map(str::to_string))
                .map(|d| {
                    if d == "uas" {
                        "UAS".into()
                    } else {
                        "BOT".into()
                    }
                });
        }
        if dir.join("idVendor").exists() {
            let hex = |f| read(&dir, f).and_then(|s| u16::from_str_radix(&s, 16).ok());
            let (vendor_id, product_id) = (hex("idVendor")?, hex("idProduct")?);
            return Some(UsbBridge {
                vendor_id,
                product_id,
                vendor_name: read(&dir, "manufacturer"),
                product_name: read(&dir, "product"),
                serial: read(&dir, "serial"),
                chipset: crate::native::bridges::chipset(vendor_id, product_id),
                transport,
                link_speed: read(&dir, "speed").map(|s| format!("{s} Mb/s")),
            });
        }
        if !dir.pop() || dir == Path::new("/sys/devices") {
            return None;
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub fn lookup(_device: &str) -> Option<UsbBridge> {
    None
}
