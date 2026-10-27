//! Runs the read-only ATA command sequence against a SAT device.

use anyhow::{Context, Result, bail};

use super::ata::{self, DevStatPage, Identify, SctStatus, SelfTestLog, SmartData};
use super::bridges::Quirks;
use super::sat::{AtaCmd, PtSize, Sat, SatError};
use super::scsi::{Inquiry, ScsiTransport};

/// Highest device statistics page with a known layout.
const MAX_DEVSTAT_PAGE: u8 = 7;
/// Extended self-test log pages to read (19 entries each).
const MAX_EXT_SELFTEST_PAGES: u16 = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub text: String,
    /// smartctl severity: "information", "warning" or "error".
    pub severity: &'static str,
}

#[derive(Debug, Clone)]
pub struct Collected {
    pub inquiry: Option<Inquiry>,
    pub identify: Identify,
    pub pt_size: PtSize,
    pub smart: Option<SmartData>,
    pub thresholds: Option<Vec<(u8, u8)>>,
    pub smart_status: Option<bool>,
    /// True if `smart_status` was derived from attribute thresholds.
    pub smart_status_from_attrs: bool,
    pub selftest: Option<SelfTestLog>,
    pub ext_selftest: Option<SelfTestLog>,
    /// (revision, device error count) from the summary SMART error log.
    pub error_log: Option<(u8, u16)>,
    pub devstats: Vec<DevStatPage>,
    pub sct: Option<SctStatus>,
    pub messages: Vec<Message>,
}

impl Collected {
    fn info(&mut self, text: impl Into<String>) {
        self.messages.push(Message {
            text: text.into(),
            severity: "information",
        });
    }

    fn warn(&mut self, text: impl Into<String>) {
        self.messages.push(Message {
            text: text.into(),
            severity: "warning",
        });
    }

    /// Unwraps an optional read: drive aborts become information, other
    /// failures become warnings.
    fn optional<R>(&mut self, what: &str, r: Result<R>) -> Option<R> {
        match r {
            Ok(v) => Some(v),
            Err(e) if matches!(e.downcast_ref::<SatError>(), Some(SatError::AtaAborted(_))) => {
                self.info(format!("{what}: not supported by drive"));
                None
            }
            Err(e) => {
                self.warn(format!("{what} failed: {e:#}"));
                None
            }
        }
    }
}

/// Reads identity, SMART data and logs. Only IDENTIFY failing is fatal.
pub fn collect<T: ScsiTransport>(sat: &mut Sat<T>, quirks: Quirks) -> Result<Collected> {
    let inquiry = sat.inquiry();
    let raw = sat.read(AtaCmd::Identify).context(
        "ATA IDENTIFY DEVICE through SAT failed; the bridge may not support ATA pass-through",
    )?;
    let identify = ata::parse_identify(&raw);
    if identify.model.is_empty() && identify.serial.is_empty() {
        bail!("IDENTIFY DEVICE returned blank data; the bridge does not pass ATA commands through");
    }

    let mut c = Collected {
        inquiry: None,
        identify,
        pt_size: sat.size,
        smart: None,
        thresholds: None,
        smart_status: None,
        smart_status_from_attrs: false,
        selftest: None,
        ext_selftest: None,
        error_log: None,
        devstats: Vec::new(),
        sct: None,
        messages: Vec::new(),
    };
    c.inquiry = c.optional("SCSI INQUIRY", inquiry);
    if c.identify.checksum_ok == Some(false) {
        c.warn("IDENTIFY DEVICE checksum mismatch");
    }
    let id = c.identify.clone();

    let gp_dir = if id.gpl_supported {
        let r = sat.read(AtaCmd::ReadLogExt(0x00, 0, 1));
        c.optional("GP log directory", r)
            .map(|b| ata::parse_log_directory(&b))
    } else {
        None
    };
    let mut smart_dir = None;

    if id.smart_supported {
        if !id.smart_enabled {
            c.warn("SMART is disabled on this drive (ddreport never enables it)");
        }
        let r = sat.read(AtaCmd::SmartReadData);
        c.smart = c
            .optional("SMART READ DATA", r)
            .map(|b| ata::parse_smart_data(&b));
        if c.smart.as_ref().is_some_and(|s| !s.checksum_ok) {
            c.warn("SMART data checksum mismatch");
        }
        let r = sat.read(AtaCmd::SmartReadThresholds);
        c.thresholds = c
            .optional("SMART READ THRESHOLDS", r)
            .map(|b| ata::parse_thresholds(&b));
        let r = sat.read(AtaCmd::SmartReadLog(0x00, 1));
        smart_dir = c
            .optional("SMART log directory", r)
            .map(|b| ata::parse_log_directory(&b));
    } else {
        c.warn("SMART is not supported by this drive");
    }

    let smart_ok = id.smart_supported && c.smart.is_some();
    let gp_pages = |addr: usize| gp_dir.as_ref().map_or(0, |d| d[addr]);
    let smart_pages = |addr: usize| smart_dir.as_ref().map_or(0, |d| d[addr]);

    if smart_ok
        && (id.self_test_supported || c.smart.as_ref().is_some_and(|s| s.self_tests_supported()))
    {
        let r = sat.read(AtaCmd::SmartReadLog(0x06, 1));
        c.selftest = c
            .optional("SMART self-test log", r)
            .map(|b| ata::parse_selftest_log(&b));
    }

    let ext_total = gp_pages(0x07);
    if ext_total > 0 {
        read_ext_selftest(sat, &mut c, ext_total);
    }

    if smart_ok
        && (id.error_log_supported
            || c.smart
                .as_ref()
                .is_some_and(|s| s.error_logging_supported()))
    {
        let r = sat.read(AtaCmd::SmartReadLog(0x01, 1));
        c.error_log = c
            .optional("SMART error log", r)
            .map(|b| ata::parse_error_log_count(&b));
    }

    read_devstats(sat, &mut c, gp_pages(0x04), smart_pages(0x04));

    if id.sct_supported && (smart_ok || id.gpl_supported) {
        let cmd = if smart_ok {
            AtaCmd::SmartReadLog(0xe0, 1)
        } else {
            AtaCmd::ReadLogExt(0xe0, 0, 1)
        };
        let r = sat.read(cmd);
        c.sct = c
            .optional("SCT status", r)
            .and_then(|b| ata::parse_sct_status(&b));
    }

    // Last: the one non-data command. Some bridges wedge on it, and everything
    // above is worth more than the status bit.
    if id.smart_supported && !quirks.no_return_status {
        c.smart_status = match sat.non_data(AtaCmd::SmartReturnStatus) {
            Ok(Some(a)) => match (a.lba_mid(), a.lba_high()) {
                (0x4f, 0xc2) => Some(true),
                (0xf4, 0x2c) => Some(false),
                (m, h) => {
                    c.warn(format!(
                        "SMART RETURN STATUS: unexpected registers 0x{m:02x}/0x{h:02x}"
                    ));
                    None
                }
            },
            Ok(None) => {
                c.warn("SMART RETURN STATUS: bridge returned no ATA registers");
                None
            }
            Err(e) => {
                c.warn(format!("SMART RETURN STATUS failed: {e:#}"));
                None
            }
        };
    }
    if c.smart_status.is_none()
        && let (Some(sd), Some(th)) = (&c.smart, &c.thresholds)
    {
        let failing = sd.attrs.iter().any(|a| {
            let t = th.iter().find(|t| t.0 == a.id).map_or(0, |t| t.1);
            a.flags & 1 != 0 && ata::when_failed(a.value, a.worst, t) == "now"
        });
        c.smart_status = Some(!failing);
        c.smart_status_from_attrs = true;
        c.warn(if quirks.no_return_status {
            "SMART RETURN STATUS skipped (this bridge stops responding to it); overall \
             status derived from attribute thresholds"
        } else {
            "SMART overall status derived from attribute thresholds"
        });
    }

    Ok(c)
}

/// Extended self-test log: page 0 (header and index), then the pages holding
/// the newest descriptors, walking backwards with wraparound.
fn read_ext_selftest<T: ScsiTransport>(sat: &mut Sat<T>, c: &mut Collected, total: u16) {
    const WHAT: &str = "Extended self-test log";
    let r = sat.read(AtaCmd::ReadLogExt(0x07, 0, 1));
    let Some(p0) = c.optional(WHAT, r) else {
        return;
    };
    let index = u16::from_le_bytes([p0[2], p0[3]]);
    if index as usize > total as usize * ata::EXT_SELFTEST_PER_PAGE {
        c.warn(format!(
            "{WHAT}: descriptor index {index} is beyond the {total}-page log"
        ));
    }
    let mut pages = std::collections::BTreeMap::from([(0u16, p0.clone())]);
    for page in ata::ext_selftest_pages_to_read(index, total, MAX_EXT_SELFTEST_PAGES) {
        if pages.contains_key(&page) {
            continue;
        }
        let r = sat.read(AtaCmd::ReadLogExt(0x07, page, 1));
        match c.optional(WHAT, r) {
            Some(b) => pages.insert(page, b),
            None => break,
        };
    }
    c.ext_selftest = Some(ata::parse_ext_selftest_pages(&p0, total, &pages));
}

/// Device statistics via READ LOG EXT (preferred) or SMART READ LOG.
fn read_devstats<T: ScsiTransport>(
    sat: &mut Sat<T>,
    c: &mut Collected,
    gp_pages: u16,
    smart_pages: u16,
) {
    const WHAT: &str = "Device statistics";
    if gp_pages > 0 {
        let r = sat.read(AtaCmd::ReadLogExt(0x04, 0, 1));
        let Some(p0) = c.optional(WHAT, r) else {
            return;
        };
        for page in ata::parse_devstat_page_list(&p0) {
            if page > MAX_DEVSTAT_PAGE || page as u16 >= gp_pages {
                continue;
            }
            let r = sat.read(AtaCmd::ReadLogExt(0x04, page as u16, 1));
            if let Some(pg) = c
                .optional(WHAT, r)
                .and_then(|b| ata::parse_devstat_page(&b, page))
            {
                c.devstats.push(pg);
            }
        }
    } else if smart_pages > 0 {
        let n = smart_pages.min(MAX_DEVSTAT_PAGE as u16 + 1) as u8;
        let r = sat.read(AtaCmd::SmartReadLog(0x04, n));
        let Some(all) = c.optional(WHAT, r) else {
            return;
        };
        for page in ata::parse_devstat_page_list(&all[..512]) {
            if page < n {
                let b = &all[512 * page as usize..512 * (page as usize + 1)];
                c.devstats.extend(ata::parse_devstat_page(b, page));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::testutil::*;

    #[test]
    fn full_collection() {
        let mut sat = Sat::new(MockDrive::default());
        let c = collect(&mut sat, Quirks::default()).unwrap();
        assert_eq!(c.inquiry.as_ref().unwrap().vendor, "Innostor");
        assert_eq!(c.identify.model, "WDC WD80EFAX-68KNBN0");
        assert_eq!(c.smart_status, Some(true));
        assert!(!c.smart_status_from_attrs);
        assert_eq!(c.smart.as_ref().unwrap().attrs.len(), 7);
        assert_eq!(c.selftest.as_ref().unwrap().entries.len(), 2);
        assert_eq!(c.ext_selftest.as_ref().unwrap().entries.len(), 2);
        assert_eq!(c.error_log, Some((1, 0)));
        let pages: Vec<u8> = c.devstats.iter().map(|p| p.number).collect();
        assert_eq!(pages, [1, 3, 5]);
        assert_eq!(c.sct.as_ref().unwrap().lifetime_max, Some(52));
        assert_eq!(c.pt_size, PtSize::Sixteen);
        assert!(c.messages.is_empty(), "{:?}", c.messages);
    }

    #[test]
    fn falls_back_to_pt12() {
        let mut sat = Sat::new(MockDrive {
            reject_pt16: true,
            ..Default::default()
        });
        let c = collect(&mut sat, Quirks::default()).unwrap();
        assert_eq!(sat.size, PtSize::Twelve);
        assert_eq!(c.smart_status, Some(true));
        assert_eq!(c.selftest.as_ref().unwrap().entries.len(), 2);
        assert!(sat.t.cdbs.iter().skip(3).all(|cdb| cdb[0] == 0xa1));
    }

    #[test]
    fn aborted_return_status_falls_back_to_thresholds() {
        let drive = MockDrive {
            status_aborted: true,
            ..Default::default()
        };
        let c = collect(&mut Sat::new(drive), Quirks::default()).unwrap();
        assert_eq!(c.smart_status, Some(true));
        assert!(c.smart_status_from_attrs);
        assert!(
            c.messages
                .iter()
                .any(|m| m.severity == "warning" && m.text.contains("aborted"))
        );
    }

    fn is_return_status(cdb: &[u8]) -> bool {
        cdb[0] == 0x85 && cdb[4] == 0xda && cdb[14] == 0xb0
    }

    #[test]
    fn return_status_runs_last() {
        let mut sat = Sat::new(MockDrive::default());
        collect(&mut sat, Quirks::default()).unwrap();
        let cdbs = &sat.t.cdbs;
        let pos = cdbs
            .iter()
            .position(|c| is_return_status(c))
            .expect("issued");
        let later_ata = cdbs[pos + 1..]
            .iter()
            .filter(|c| c[0] == 0x85 && c[14] != 0x03)
            .count();
        assert_eq!(later_ata, 0, "only REQUEST SENSE may follow it");
    }

    #[test]
    fn quirk_skips_return_status() {
        let mut sat = Sat::new(MockDrive::default());
        let c = collect(
            &mut sat,
            Quirks {
                no_return_status: true,
            },
        )
        .unwrap();
        assert!(!sat.t.cdbs.iter().any(|c| is_return_status(c)));
        assert_eq!(c.smart_status, Some(true));
        assert!(c.smart_status_from_attrs);
        assert!(c.messages.iter().any(|m| m.text.contains("skipped")));
    }

    #[test]
    fn ext_selftest_reads_newest_pages() {
        let entries: Vec<_> = (95..=98).map(|i| (i, 0x01, 0x00, i as u16, 0)).collect();
        let drive = MockDrive {
            ext_selftest_log: ext_selftest_log_pages(8, &entries, 98),
            gp_dir: log_directory_sector(&[(0x04, 8), (0x07, 8), (0xe0, 1)]),
            ..Default::default()
        };
        let mut sat = Sat::new(drive);
        let c = collect(&mut sat, Quirks::default()).unwrap();
        let hours: Vec<u16> = c
            .ext_selftest
            .unwrap()
            .entries
            .iter()
            .map(|e| e.hours)
            .collect();
        assert_eq!(hours, [98, 97, 96, 95]);
        let pages: Vec<u16> = sat
            .t
            .cdbs
            .iter()
            .filter(|cdb| cdb[0] == 0x85 && cdb[14] == 0x2f && cdb[8] == 0x07)
            .map(|cdb| cdb[10] as u16 | (cdb[11] as u16) << 8)
            .collect();
        assert_eq!(pages, [0, 5, 4, 3, 2]);
    }

    #[test]
    fn failing_drive() {
        let mut sat = Sat::new(MockDrive {
            failing: true,
            ..Default::default()
        });
        assert_eq!(
            collect(&mut sat, Quirks::default()).unwrap().smart_status,
            Some(false)
        );
    }

    #[test]
    fn non_gpl_drive_uses_smart_logs() {
        let spec = IdentifySpec {
            gpl: false,
            ..Default::default()
        };
        let mut sat = Sat::new(MockDrive {
            identify: identify_sector(&spec),
            ..Default::default()
        });
        let c = collect(&mut sat, Quirks::default()).unwrap();
        assert!(c.ext_selftest.is_none());
        let pages: Vec<u8> = c.devstats.iter().map(|p| p.number).collect();
        assert_eq!(pages, [1, 3, 5]);
        assert!(
            sat.t
                .cdbs
                .iter()
                .all(|cdb| cdb[0] != 0x85 || cdb[14] != 0x2f)
        );
    }

    #[test]
    fn empty_log_directories_skip_logs() {
        let drive = MockDrive {
            smart_dir: log_directory_sector(&[]),
            gp_dir: log_directory_sector(&[]),
            ..Default::default()
        };
        let c = collect(&mut Sat::new(drive), Quirks::default()).unwrap();
        assert!(c.devstats.is_empty());
        assert!(c.ext_selftest.is_none());
        assert!(
            c.selftest.is_some(),
            "self-test log read is gated by capability bits"
        );
        assert!(c.messages.is_empty(), "{:?}", c.messages);
    }

    #[test]
    fn ata_abort_is_information() {
        let drive = MockDrive {
            smart_dir: log_directory_sector(&[(0x04, 9)]),
            gp_dir: log_directory_sector(&[]),
            devstats: Vec::new(),
            ..Default::default()
        };
        let c = collect(&mut Sat::new(drive), Quirks::default()).unwrap();
        assert_eq!(c.messages.len(), 1);
        assert_eq!(c.messages[0].severity, "information");
    }

    #[test]
    fn blank_identify_is_fatal() {
        let mut sat = Sat::new(MockDrive {
            identify: vec![0; 512],
            ..Default::default()
        });
        assert!(collect(&mut sat, Quirks::default()).is_err());
    }

    #[test]
    fn only_read_only_commands_sent() {
        let mut sat = Sat::new(MockDrive::default());
        collect(&mut sat, Quirks::default()).unwrap();
        for cdb in &sat.t.cdbs {
            let (cmd, feat) = match cdb[0] {
                0x85 => (cdb[14], cdb[4]),
                0xa1 => (cdb[9], cdb[3]),
                0x12 | 0x03 => continue,
                op => panic!("unexpected SCSI opcode 0x{op:02x}"),
            };
            assert!(
                matches!(
                    (cmd, feat),
                    (0xec, _) | (0x2f, _) | (0xb0, 0xd0 | 0xd1 | 0xd5 | 0xda)
                ),
                "non read-only ATA command 0x{cmd:02x}/0x{feat:02x}"
            );
        }
    }
}
