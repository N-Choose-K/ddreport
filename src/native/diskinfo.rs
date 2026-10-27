//! What the OS knows about the disk behind a USB bridge, read without touching
//! the device: BSD name, media name, size and mounted volumes. macOS only.

use super::UsbStorageDevice;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbDisk {
    /// Whole physical disk, e.g. "disk4".
    pub bsd: String,
    pub media_name: Option<String>,
    pub size_bytes: Option<u64>,
    /// Mount points of its volumes, including APFS volumes synthesized from it.
    pub mounts: Vec<String>,
}

/// Whole disks ("disk4") among `names`, in registry order: the physical disk
/// first, synthesized APFS disks after it.
pub fn whole_disks(names: &[String]) -> Vec<String> {
    names
        .iter()
        .filter(|n| {
            n.strip_prefix("disk")
                .is_some_and(|r| !r.is_empty() && r.bytes().all(|b| b.is_ascii_digit()))
        })
        .cloned()
        .collect()
}

/// Media name and size from `diskutil info -plist`.
pub fn parse_diskutil(xml: &[u8]) -> (Option<String>, Option<u64>) {
    let Ok(plist::Value::Dictionary(d)) = plist::Value::from_reader_xml(xml) else {
        return (None, None);
    };
    let name = d
        .get("MediaName")
        .and_then(plist::Value::as_string)
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());
    let size = d.get("TotalSize").or_else(|| d.get("Size")).and_then(|v| {
        v.as_unsigned_integer()
            .or_else(|| v.as_signed_integer().map(|x| x as u64))
    });
    (name, size)
}

#[cfg(target_os = "macos")]
pub fn disk_info(dev: &UsbStorageDevice) -> Option<UsbDisk> {
    use std::process::Command;

    use super::busy;

    let names = busy::macos_disks(dev).ok()?;
    let bsd = whole_disks(&names).into_iter().next()?;
    let (media_name, size_bytes) = Command::new("/usr/sbin/diskutil")
        .args(["info", "-plist", &bsd])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| parse_diskutil(&o.stdout))
        .unwrap_or((None, None));
    let mounts = Command::new("/sbin/mount")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            busy::parse_bsd_mount(&String::from_utf8_lossy(&o.stdout))
                .into_iter()
                .filter(|m| busy::macos_belongs(&m.source, &names))
                .map(|m| m.target)
                .collect()
        })
        .unwrap_or_default();
    Some(UsbDisk {
        bsd,
        media_name,
        size_bytes,
        mounts,
    })
}

#[cfg(not(target_os = "macos"))]
pub fn disk_info(_dev: &UsbStorageDevice) -> Option<UsbDisk> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_disks_keeps_registry_order() {
        let names: Vec<String> = ["disk6", "disk6s1", "disk6s2", "disk7", "rdisk8", "disk"]
            .map(String::from)
            .to_vec();
        assert_eq!(whole_disks(&names), ["disk6", "disk7"]);
    }

    #[test]
    fn diskutil_plist_fields() {
        let xml = br#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
  <key>MediaName</key><string> CT1050MX300SSD1 </string>
  <key>TotalSize</key><integer>1050214588416</integer>
</dict></plist>"#;
        assert_eq!(
            parse_diskutil(xml),
            (Some("CT1050MX300SSD1".into()), Some(1050214588416))
        );
        assert_eq!(parse_diskutil(b"junk"), (None, None));
    }
}
