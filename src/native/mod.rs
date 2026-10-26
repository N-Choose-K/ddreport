//! Native USB-SATA backend: ATA pass-through (SAT) over USB Mass Storage
//! Bulk-Only Transport, for drives smartctl cannot reach (USB on macOS).
//! Output is a subset of `smartctl -x --json` so the smartctl parser reads it.
//!
//! # Everything this module sends to a device (all read-only)
//! - USB control requests: GET_MAX_LUN, Bulk-Only Mass Storage Reset,
//!   CLEAR_FEATURE(ENDPOINT_HALT), and SET_INTERFACE only on UAS devices to
//!   select the Bulk-Only alternate setting.
//! - SCSI: INQUIRY, REQUEST SENSE, ATA PASS-THROUGH(16)/(12).
//! - ATA (inside PASS-THROUGH): IDENTIFY DEVICE (0xEC), SMART READ DATA
//!   (0xB0/0xD0), SMART READ THRESHOLDS (0xB0/0xD1), SMART READ LOG
//!   (0xB0/0xD5), SMART RETURN STATUS (0xB0/0xDA), READ LOG EXT (0x2F).
//!
//! No write, self-test start/abort, SMART enable/disable, security, feature
//! or other state-changing command exists in this code: `sat::AtaCmd` only
//! has the variants above, and `scsi::ScsiTransport` has no data-out path.
//!
//! Detaching the OS driver makes the disk disappear from the OS, so `busy`
//! refuses first if the disk is mounted, swapped on, or open in any process.
//! macOS requires root (libusb captures the device via USBDeviceReEnumerate).

pub mod ata;
pub mod attr_names;
pub mod bot;
pub mod bridges;
pub mod busy;
pub mod collect;
pub mod diskinfo;
pub mod json;
pub mod sat;
pub mod scsi;
pub mod selftest;
pub mod usb;

#[cfg(test)]
mod testutil;

use std::fmt;
use std::str::FromStr;

use anyhow::{Result, anyhow, bail};

use crate::model::UsbBridge;
use collect::Message;
pub use diskinfo::{UsbDisk, disk_info};
use sat::PtSize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    /// Bulk-Only Transport (interface protocol 0x50).
    Bot,
    /// USB Attached SCSI (interface protocol 0x62).
    Uas,
}

impl Transport {
    pub fn as_str(self) -> &'static str {
        match self {
            Transport::Bot => "BOT",
            Transport::Uas => "UAS",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbStorageDevice {
    pub bus: u8,
    pub address: u8,
    /// macOS IORegistry locationID (bus and port path); survives re-enumeration,
    /// which `address` does not. 0 elsewhere.
    pub location: u32,
    pub vendor_id: u16,
    pub product_id: u16,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    pub serial: Option<String>,
    pub transport: Transport,
    pub speed: Option<String>,
    /// Mass-storage interface number.
    pub interface: u8,
}

impl UsbStorageDevice {
    /// Selector string that picks this device: "vid:pid[:serial]".
    pub fn selector(&self) -> String {
        self.vid_pid_selector().to_string()
    }

    /// Selector by IDs and serial (when the device reports one).
    pub fn vid_pid_selector(&self) -> UsbSelector {
        UsbSelector::VidPid {
            vid: self.vendor_id,
            pid: self.product_id,
            serial: self.serial.clone().filter(|s| !s.is_empty()),
        }
    }

    /// Selector that singles this device out among `all`: IDs and serial when
    /// unique, otherwise bus and address.
    pub fn unique_selector(&self, all: &[UsbStorageDevice]) -> UsbSelector {
        let sel = self.vid_pid_selector();
        if all.iter().filter(|d| sel.matches(d)).count() == 1 {
            sel
        } else {
            UsbSelector::BusAddr {
                bus: self.bus,
                address: self.address,
            }
        }
    }
}

impl fmt::Display for UsbStorageDevice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:03}:{:03} {:04x}:{:04x}",
            self.bus, self.address, self.vendor_id, self.product_id
        )?;
        for s in [&self.manufacturer, &self.product].into_iter().flatten() {
            write!(f, " {s}")?;
        }
        if let Some(s) = &self.serial {
            write!(f, " serial={s}")?;
        }
        write!(f, " [{}]", self.transport.as_str())?;
        if let Some(s) = &self.speed {
            write!(f, " {s}")?;
        }
        Ok(())
    }
}

/// Picks a USB device: "vvvv:pppp", "vvvv:pppp:SERIAL" (hex IDs) or "bus:addr" (decimal).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UsbSelector {
    VidPid {
        vid: u16,
        pid: u16,
        serial: Option<String>,
    },
    BusAddr {
        bus: u8,
        address: u8,
    },
}

impl FromStr for UsbSelector {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let s = s.trim();
        let mut parts = s.splitn(3, ':');
        let (a, b) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
        let serial = parts.next();
        let hex4 = |x: &str| x.len() == 4 && x.chars().all(|c| c.is_ascii_hexdigit());
        if hex4(a) && hex4(b) {
            return Ok(UsbSelector::VidPid {
                vid: u16::from_str_radix(a, 16)?,
                pid: u16::from_str_radix(b, 16)?,
                serial: match serial {
                    Some("") => bail!("empty serial in USB selector {s:?}"),
                    other => other.map(str::to_owned),
                },
            });
        }
        if serial.is_none()
            && let (Ok(bus), Ok(address)) = (a.parse(), b.parse())
        {
            return Ok(UsbSelector::BusAddr { bus, address });
        }
        Err(anyhow!(
            "invalid USB selector {s:?}; expected vid:pid, vid:pid:serial or bus:addr"
        ))
    }
}

impl fmt::Display for UsbSelector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UsbSelector::VidPid {
                vid,
                pid,
                serial: Some(s),
            } => write!(f, "{vid:04x}:{pid:04x}:{s}"),
            UsbSelector::VidPid {
                vid,
                pid,
                serial: None,
            } => write!(f, "{vid:04x}:{pid:04x}"),
            UsbSelector::BusAddr { bus, address } => write!(f, "{bus}:{address}"),
        }
    }
}

impl UsbSelector {
    pub fn matches(&self, d: &UsbStorageDevice) -> bool {
        match self {
            UsbSelector::VidPid { vid, pid, serial } => {
                d.vendor_id == *vid
                    && d.product_id == *pid
                    && serial.as_ref().is_none_or(|s| d.serial.as_ref() == Some(s))
            }
            UsbSelector::BusAddr { bus, address } => d.bus == *bus && d.address == *address,
        }
    }
}

/// Exactly one device matching `sel`.
pub fn select<'a>(devs: &'a [UsbStorageDevice], sel: &UsbSelector) -> Result<&'a UsbStorageDevice> {
    let hits: Vec<_> = devs.iter().filter(|d| sel.matches(d)).collect();
    match hits.as_slice() {
        [one] => Ok(one),
        [] => bail!("no USB mass-storage device matches {sel}"),
        many => bail!(
            "{} devices match {sel}; use vid:pid:serial or bus:addr:\n{}",
            many.len(),
            many.iter()
                .map(|d| format!("  {d}"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    }
}

pub struct NativeResult {
    /// Subset of `smartctl -x --json` output.
    pub json: serde_json::Value,
    pub bridge: UsbBridge,
    /// Bridge SCSI INQUIRY "vendor product revision".
    pub bridge_inquiry: Option<String>,
}

pub fn bridge_info(d: &UsbStorageDevice) -> UsbBridge {
    UsbBridge {
        vendor_id: d.vendor_id,
        product_id: d.product_id,
        vendor_name: d
            .manufacturer
            .clone()
            .or_else(|| bridges::vendor(d.vendor_id).map(str::to_owned)),
        product_name: d.product.clone(),
        serial: d.serial.clone(),
        chipset: bridges::chipset(d.vendor_id, d.product_id),
        transport: Some(d.transport.as_str().to_owned()),
        link_speed: d.speed.clone(),
    }
}

/// True when running with effective UID 0.
pub fn is_root() -> bool {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() == 0 }
}

/// Lists USB mass-storage devices from OS-cached descriptors. Sends nothing
/// to any device.
pub fn list_devices() -> Result<Vec<UsbStorageDevice>> {
    usb::list_devices()
}

pub use selftest::{Expected, TestOutcome};

/// Starts (`kind` set) or aborts a self-test on the selected USB-SATA drive.
/// Same capture rules as `read_smart`.
pub fn self_test(
    sel: &UsbSelector,
    kind: Option<crate::smartctl::TestKind>,
    expected: Option<&Expected>,
    force: bool,
    unmount: bool,
) -> Result<TestOutcome> {
    let devs = list_devices()?;
    let dev = select(&devs, sel)?;
    let (out, _) = usb::with_sat(dev, force, unmount, |sat| {
        selftest::run(sat, kind, expected)
    })?;
    Ok(out)
}

/// Reads identity and SMART data from the selected USB-SATA drive. Refuses if
/// its disk is mounted or open; `unmount` unmounts its volumes first (macOS
/// remounts them after the read); `force` only skips checks that can't be done.
pub fn read_smart(sel: &UsbSelector, force: bool, unmount: bool) -> Result<NativeResult> {
    let devs = list_devices()?;
    let dev = select(&devs, sel)?;
    // The closure lets the pipe lifetime be higher-ranked; a fn path can't.
    #[allow(clippy::redundant_closure)]
    let quirks = bridges::quirks(dev.vendor_id, dev.product_id);
    let (mut c, notes) = usb::with_sat(dev, force, unmount, |sat| collect::collect(sat, quirks))?;
    c.messages.extend(notes);
    if c.pt_size == PtSize::Twelve {
        c.messages.push(Message {
            text: "bridge rejected ATA PASS-THROUGH(16); used PASS-THROUGH(12)".into(),
            severity: "information",
        });
    }
    let bridge = bridge_info(dev);
    let name = dev.selector();
    let info_name = match &bridge.chipset {
        Some(chip) => format!("{name} [{chip}] [SAT native]"),
        None => format!("{name} [SAT native]"),
    };
    Ok(NativeResult {
        json: json::build(&c, &name, &info_name),
        bridge,
        bridge_inquiry: c.inquiry.as_ref().map(ToString::to_string),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(bus: u8, address: u8, serial: Option<&str>) -> UsbStorageDevice {
        UsbStorageDevice {
            bus,
            address,
            location: 0,
            vendor_id: 0x1f75,
            product_id: 0x0611,
            manufacturer: None,
            product: Some("Ext. HDD".into()),
            serial: serial.map(str::to_owned),
            transport: Transport::Bot,
            speed: None,
            interface: 0,
        }
    }

    #[test]
    fn parse_selectors() {
        assert_eq!(
            "1f75:0611".parse::<UsbSelector>().unwrap(),
            UsbSelector::VidPid {
                vid: 0x1f75,
                pid: 0x0611,
                serial: None
            }
        );
        assert_eq!(
            "152D:0578:ABC:123".parse::<UsbSelector>().unwrap(),
            UsbSelector::VidPid {
                vid: 0x152d,
                pid: 0x0578,
                serial: Some("ABC:123".into())
            }
        );
        assert_eq!(
            "2:7".parse::<UsbSelector>().unwrap(),
            UsbSelector::BusAddr { bus: 2, address: 7 }
        );
        assert_eq!(
            "020:017".parse::<UsbSelector>().unwrap(),
            UsbSelector::BusAddr {
                bus: 20,
                address: 17
            }
        );
        for bad in [
            "",
            "1f75",
            "1f75:",
            "zzzz:0611",
            "1f75:0611:",
            "300:1",
            "1:2:3",
        ] {
            assert!(bad.parse::<UsbSelector>().is_err(), "{bad:?} should fail");
        }
        for s in ["1f75:0611", "1f75:0611:XYZ", "3:4"] {
            assert_eq!(s.parse::<UsbSelector>().unwrap().to_string(), s);
        }
    }

    #[test]
    fn selection() {
        let devs = [dev(1, 5, Some("A")), dev(2, 3, Some("B"))];
        let sel = |s: &str| s.parse::<UsbSelector>().unwrap();
        assert!(select(&devs, &sel("1f75:0611")).is_err(), "ambiguous");
        assert_eq!(select(&devs, &sel("1f75:0611:B")).unwrap().bus, 2);
        assert_eq!(
            select(&devs, &sel("1:5")).unwrap().serial.as_deref(),
            Some("A")
        );
        assert!(select(&devs, &sel("152d:0578")).is_err());
        assert_eq!(devs[0].selector(), "1f75:0611:A");
    }

    #[test]
    fn bridge_fields() {
        let b = bridge_info(&dev(1, 5, Some("A")));
        assert_eq!(b.chipset.as_deref(), Some("Innostor IS611"));
        assert_eq!(b.vendor_name.as_deref(), Some("Innostor"));
        assert_eq!(b.transport.as_deref(), Some("BOT"));
    }
}
