//! USB access: enumeration via nusb (OS-cached descriptors and strings, no
//! device I/O) and bulk I/O via libusb (rusb), which can detach the kernel
//! mass-storage driver on Linux and capture the device on macOS (root only).

use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use nusb::MaybeFuture;
use rusb::UsbContext;

use super::bot::{Bot, BulkPipe, PipeError};
use super::busy;
use super::collect::Message;
use super::sat::Sat;
use super::{Transport, UsbStorageDevice};

const CLASS_MASS_STORAGE: u8 = 0x08;
const SUBCLASS_SCSI: u8 = 0x06;
const PROTO_BOT: u8 = 0x50;
const PROTO_UAS: u8 = 0x62;

const CTRL_TIMEOUT: Duration = Duration::from_secs(5);

/// Lists mass-storage devices from OS-cached data. Opens nothing.
pub fn list_devices() -> Result<Vec<UsbStorageDevice>> {
    let mut out = Vec::new();
    for info in nusb::list_devices()
        .wait()
        .context("enumerating USB devices")?
    {
        let Some(iface) = info.interfaces().find(|i| {
            i.class() == CLASS_MASS_STORAGE
                && i.subclass() == SUBCLASS_SCSI
                && matches!(i.protocol(), PROTO_BOT | PROTO_UAS)
        }) else {
            continue;
        };
        out.push(UsbStorageDevice {
            bus: bus_number(&info),
            address: info.device_address(),
            vendor_id: info.vendor_id(),
            product_id: info.product_id(),
            manufacturer: info.manufacturer_string().map(str::to_owned),
            product: info.product_string().map(str::to_owned),
            serial: info.serial_number().map(str::to_owned),
            transport: if iface.protocol() == PROTO_UAS {
                Transport::Uas
            } else {
                Transport::Bot
            },
            speed: info.speed().map(|s| speed_name(s).to_owned()),
            interface: iface.interface_number(),
        });
    }
    Ok(out)
}

/// Bus number as libusb reports it.
fn bus_number(info: &nusb::DeviceInfo) -> u8 {
    #[cfg(target_os = "linux")]
    {
        info.busnum()
    }
    #[cfg(target_os = "macos")]
    {
        (info.location_id() >> 24) as u8
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        info.bus_id().parse().unwrap_or(0)
    }
}

fn speed_name(s: nusb::Speed) -> &'static str {
    match s {
        nusb::Speed::Low => "USB 1.0 Low Speed (1.5 Mb/s)",
        nusb::Speed::Full => "USB 1.1 Full Speed (12 Mb/s)",
        nusb::Speed::High => "USB 2.0 High Speed (480 Mb/s)",
        nusb::Speed::Super => "USB 3.x SuperSpeed (5 Gb/s)",
        nusb::Speed::SuperPlus => "USB 3.x SuperSpeed+ (10 Gb/s)",
        _ => "unknown",
    }
}

fn pipe_err(e: rusb::Error) -> PipeError {
    match e {
        rusb::Error::Pipe => PipeError::Stall,
        rusb::Error::Timeout => PipeError::Timeout,
        e => PipeError::Other(e.to_string()),
    }
}

/// Bulk endpoints of a claimed BOT interface.
pub struct RusbPipe<'a> {
    handle: &'a rusb::DeviceHandle<rusb::Context>,
    ep_in: u8,
    ep_out: u8,
    iface: u8,
}

impl BulkPipe for RusbPipe<'_> {
    fn write_out(&mut self, data: &[u8], timeout: Duration) -> Result<usize, PipeError> {
        self.handle
            .write_bulk(self.ep_out, data, timeout)
            .map_err(pipe_err)
    }
    fn read_in(&mut self, buf: &mut [u8], timeout: Duration) -> Result<usize, PipeError> {
        self.handle
            .read_bulk(self.ep_in, buf, timeout)
            .map_err(pipe_err)
    }
    fn clear_halt_in(&mut self) -> Result<(), PipeError> {
        self.handle.clear_halt(self.ep_in).map_err(pipe_err)
    }
    fn clear_halt_out(&mut self) -> Result<(), PipeError> {
        self.handle.clear_halt(self.ep_out).map_err(pipe_err)
    }
    fn bot_reset(&mut self) -> Result<(), PipeError> {
        let rt = rusb::request_type(
            rusb::Direction::Out,
            rusb::RequestType::Class,
            rusb::Recipient::Interface,
        );
        self.handle
            .write_control(rt, 0xff, 0, self.iface as u16, &[], CTRL_TIMEOUT)
            .map(|_| ())
            .map_err(pipe_err)
    }
}

/// Claimed interface. Drop releases it and reattaches the kernel driver
/// (on macOS: re-enumerates the device so the OS driver takes it back).
struct Claim {
    handle: rusb::DeviceHandle<rusb::Context>,
    iface: u8,
    reattach: bool,
}

impl Claim {
    fn new(handle: rusb::DeviceHandle<rusb::Context>, iface: u8) -> Result<Self> {
        let reattach = match handle.kernel_driver_active(iface) {
            Ok(true) => {
                handle.detach_kernel_driver(iface).map_err(|e| match e {
                    rusb::Error::Access => anyhow!(
                        "detaching the OS mass-storage driver was denied; run as root \
                         (macOS device capture needs root or the com.apple.vm.device-access entitlement)"
                    ),
                    e => anyhow!("detaching kernel driver: {e}"),
                })?;
                true
            }
            _ => false,
        };
        let claim = Claim {
            handle,
            iface,
            reattach,
        };
        claim
            .handle
            .claim_interface(iface)
            .with_context(|| format!("claiming interface {iface}"))?;
        Ok(claim)
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        let _ = self.handle.release_interface(self.iface);
        if self.reattach {
            let _ = self.handle.attach_kernel_driver(self.iface);
        }
    }
}

/// BOT alternate setting and its bulk endpoints.
fn find_bot_alt(cfg: &rusb::ConfigDescriptor, iface: u8) -> Option<(u8, u8, u8)> {
    let i = cfg.interfaces().find(|i| i.number() == iface)?;
    i.descriptors().find_map(|alt| {
        if alt.class_code() != CLASS_MASS_STORAGE
            || alt.sub_class_code() != SUBCLASS_SCSI
            || alt.protocol_code() != PROTO_BOT
        {
            return None;
        }
        let bulk = |dir| {
            alt.endpoint_descriptors()
                .find(|e| e.transfer_type() == rusb::TransferType::Bulk && e.direction() == dir)
                .map(|e| e.address())
        };
        Some((
            alt.setting_number(),
            bulk(rusb::Direction::In)?,
            bulk(rusb::Direction::Out)?,
        ))
    })
}

/// Checks the disk is idle, claims the BOT interface and runs `f` against it.
/// Returns `f`'s result plus notes for the report. `force` only skips busy
/// checks that could not be completed (see `busy`).
pub fn with_sat<R>(
    dev: &UsbStorageDevice,
    force: bool,
    unmount: bool,
    f: impl FnOnce(&mut Sat<Bot<RusbPipe<'_>>>) -> Result<R>,
) -> Result<(R, Vec<Message>)> {
    let ctx = rusb::Context::new().context("initializing libusb")?;
    let device = ctx
        .devices()?
        .iter()
        .find(|d| {
            d.bus_number() == dev.bus
                && d.address() == dev.address
                && d.device_descriptor().is_ok_and(|dd| {
                    dd.vendor_id() == dev.vendor_id && dd.product_id() == dev.product_id
                })
        })
        .ok_or_else(|| {
            anyhow!(
                "device {:04x}:{:04x} at bus {} address {} not found by libusb",
                dev.vendor_id,
                dev.product_id,
                dev.bus,
                dev.address
            )
        })?;
    let cfg = device
        .active_config_descriptor()
        .context("reading configuration descriptor")?;
    let (alt, ep_in, ep_out) = find_bot_alt(&cfg, dev.interface).ok_or_else(|| {
        anyhow!(
            "interface {} has no Bulk-Only Transport alternate setting (UAS-only bridge?)",
            dev.interface
        )
    })?;

    if unmount {
        busy::unmount(dev)?;
    }
    let mut notes: Vec<Message> = busy::ensure_idle(dev, force)?
        .into_iter()
        .map(|text| Message {
            text,
            severity: "warning",
        })
        .collect();
    let info = |text: String| Message {
        text,
        severity: "information",
    };
    if unmount {
        notes.push(info(
            "volumes were unmounted for the read; the OS remounts them afterwards".into(),
        ));
    }

    let handle = device.open().map_err(|e| match e {
        rusb::Error::Access => anyhow!("permission denied opening USB device; run as root"),
        e => anyhow!("opening USB device: {e}"),
    })?;
    let claim = Claim::new(handle, dev.interface)?;
    if dev.transport == Transport::Uas || alt != 0 {
        claim
            .handle
            .set_alternate_setting(dev.interface, alt)
            .with_context(|| format!("selecting BOT alternate setting {alt}"))?;
        notes.push(info(format!(
            "UAS device; using Bulk-Only alternate setting {alt}"
        )));
    }

    let rt = rusb::request_type(
        rusb::Direction::In,
        rusb::RequestType::Class,
        rusb::Recipient::Interface,
    );
    let mut lun = [0u8; 1];
    match claim
        .handle
        .read_control(rt, 0xfe, 0, dev.interface as u16, &mut lun, CTRL_TIMEOUT)
    {
        Ok(1) if lun[0] & 0x0f > 0 => notes.push(info(format!(
            "bridge reports {} LUNs; reading LUN 0",
            (lun[0] & 0x0f) + 1
        ))),
        _ => {}
    }

    let pipe = RusbPipe {
        handle: &claim.handle,
        ep_in,
        ep_out,
        iface: dev.interface,
    };
    let mut sat = Sat::new(Bot::new(pipe, 0));
    let r = f(&mut sat)?;
    Ok((r, notes))
}
