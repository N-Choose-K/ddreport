//! SCSI/ATA Translation (SAT) ATA PASS-THROUGH(16) and (12), restricted to
//! read-only ATA commands.

use std::time::Duration;

use anyhow::{Result, bail};

use super::scsi::{self, AtaReturn, ScsiStatus, ScsiTransport, Sense};

pub const PT16: u8 = 0x85;
pub const PT12: u8 = 0xa1;

pub const ATA_IDENTIFY: u8 = 0xec;
pub const ATA_SMART: u8 = 0xb0;
pub const ATA_READ_LOG_EXT: u8 = 0x2f;

pub const SMART_READ_DATA: u8 = 0xd0;
pub const SMART_READ_THRESHOLDS: u8 = 0xd1;
pub const SMART_READ_LOG: u8 = 0xd5;
pub const SMART_RETURN_STATUS: u8 = 0xda;
pub const SMART_EXECUTE_OFFLINE: u8 = 0xd4;

const SMART_LBA: u64 = 0xc2_4f00;

const PROTO_NON_DATA: u8 = 3;
const PROTO_PIO_IN: u8 = 4;
/// T_DIR=from device, BYT_BLK=blocks, T_LENGTH=sector count field.
const FLAGS_PIO_IN: u8 = 0x08 | 0x04 | 0x02;
const CK_COND: u8 = 0x20;

pub const TIMEOUT: Duration = Duration::from_secs(20);

/// The read-only ATA commands this module can build. There is no variant
/// that writes, starts a self-test, enables features or changes state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AtaCmd {
    Identify,
    SmartReadData,
    SmartReadThresholds,
    /// SMART READ LOG: log address, sector count.
    SmartReadLog(u8, u8),
    SmartReturnStatus,
    /// SMART EXECUTE OFF-LINE IMMEDIATE: subcommand (1 short, 2 extended,
    /// 3 conveyance, 0x7f abort).
    SmartExecuteOffline(u8),
    /// READ LOG EXT: log address, first page, page count.
    ReadLogExt(u8, u16, u16),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Taskfile {
    pub command: u8,
    pub features: u16,
    pub count: u16,
    pub lba: u64,
    pub device: u8,
    /// 48-bit command (EXTEND bit).
    pub ext: bool,
}

impl AtaCmd {
    pub fn taskfile(self) -> Taskfile {
        let smart = |features: u8, lba_low: u8, count: u8| Taskfile {
            command: ATA_SMART,
            features: features as u16,
            count: count as u16,
            lba: SMART_LBA | lba_low as u64,
            ..Default::default()
        };
        match self {
            AtaCmd::Identify => Taskfile {
                command: ATA_IDENTIFY,
                count: 1,
                ..Default::default()
            },
            AtaCmd::SmartReadData => smart(SMART_READ_DATA, 0, 1),
            AtaCmd::SmartReadThresholds => smart(SMART_READ_THRESHOLDS, 0, 1),
            AtaCmd::SmartReadLog(log, n) => smart(SMART_READ_LOG, log, n),
            AtaCmd::SmartReturnStatus => smart(SMART_RETURN_STATUS, 0, 0),
            AtaCmd::SmartExecuteOffline(sub) => smart(SMART_EXECUTE_OFFLINE, sub, 0),
            AtaCmd::ReadLogExt(log, page, n) => Taskfile {
                command: ATA_READ_LOG_EXT,
                count: n,
                lba: log as u64 | ((page & 0xff) as u64) << 8 | ((page >> 8) as u64) << 40,
                ext: true,
                ..Default::default()
            },
        }
    }

    /// Bytes returned in the data-in phase.
    pub fn data_len(self) -> usize {
        match self {
            AtaCmd::SmartReturnStatus | AtaCmd::SmartExecuteOffline(_) => 0,
            _ => self.taskfile().count as usize * 512,
        }
    }
}

pub fn cdb16(tf: &Taskfile, data_in: bool, ck_cond: bool) -> [u8; 16] {
    let proto = if data_in {
        PROTO_PIO_IN
    } else {
        PROTO_NON_DATA
    };
    let mut c = [0u8; 16];
    c[0] = PT16;
    c[1] = proto << 1 | tf.ext as u8;
    c[2] = if data_in { FLAGS_PIO_IN } else { 0 } | if ck_cond { CK_COND } else { 0 };
    c[3] = (tf.features >> 8) as u8;
    c[4] = tf.features as u8;
    c[5] = (tf.count >> 8) as u8;
    c[6] = tf.count as u8;
    c[7] = (tf.lba >> 24) as u8;
    c[8] = tf.lba as u8;
    c[9] = (tf.lba >> 32) as u8;
    c[10] = (tf.lba >> 8) as u8;
    c[11] = (tf.lba >> 40) as u8;
    c[12] = (tf.lba >> 16) as u8;
    c[13] = tf.device;
    c[14] = tf.command;
    c
}

/// PASS-THROUGH(12); None if the taskfile needs 48-bit registers.
pub fn cdb12(tf: &Taskfile, data_in: bool, ck_cond: bool) -> Option<[u8; 12]> {
    if tf.features > 0xff || tf.count > 0xff || tf.lba > 0xff_ffff {
        return None;
    }
    let proto = if data_in {
        PROTO_PIO_IN
    } else {
        PROTO_NON_DATA
    };
    let mut c = [0u8; 12];
    c[0] = PT12;
    c[1] = proto << 1;
    c[2] = if data_in { FLAGS_PIO_IN } else { 0 } | if ck_cond { CK_COND } else { 0 };
    c[3] = tf.features as u8;
    c[4] = tf.count as u8;
    c[5] = tf.lba as u8;
    c[6] = (tf.lba >> 8) as u8;
    c[7] = (tf.lba >> 16) as u8;
    c[8] = tf.device;
    c[9] = tf.command;
    Some(c)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PtSize {
    Sixteen,
    Twelve,
}

/// Why a passthrough command produced no data.
#[derive(Debug)]
pub enum SatError {
    /// The drive aborted the ATA command (typically: not supported).
    AtaAborted(AtaReturn),
    /// The bridge rejected the CDB or returned unexpected sense.
    Rejected(Sense),
    /// Fewer bytes than requested.
    ShortRead(usize, usize),
}

impl std::fmt::Display for SatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SatError::AtaAborted(a) => {
                write!(
                    f,
                    "ATA command aborted (status 0x{:02x}, error 0x{:02x})",
                    a.status, a.error
                )
            }
            SatError::Rejected(s) => write!(f, "bridge rejected command ({s})"),
            SatError::ShortRead(got, want) => write!(f, "short read: {got} of {want} bytes"),
        }
    }
}

impl std::error::Error for SatError {}

/// ATA device behind a SAT bridge. Falls back from PASS-THROUGH(16) to (12)
/// if the bridge rejects 16 before any 16-byte command has succeeded.
pub struct Sat<T: ScsiTransport> {
    pub t: T,
    pub size: PtSize,
    pt16_ok: bool,
}

impl<T: ScsiTransport> Sat<T> {
    pub fn new(t: T) -> Self {
        Sat {
            t,
            size: PtSize::Sixteen,
            pt16_ok: false,
        }
    }

    fn exec(
        &mut self,
        tf: &Taskfile,
        data: &mut [u8],
        ck_cond: bool,
    ) -> Result<(ScsiStatus, usize, Option<Sense>)> {
        self.exec_within(tf, data, ck_cond, TIMEOUT)
    }

    fn exec_within(
        &mut self,
        tf: &Taskfile,
        data: &mut [u8],
        ck_cond: bool,
        timeout: Duration,
    ) -> Result<(ScsiStatus, usize, Option<Sense>)> {
        let data_in = !data.is_empty();
        loop {
            let out = match self.size {
                PtSize::Sixteen => self
                    .t
                    .execute(&cdb16(tf, data_in, ck_cond), data, timeout)?,
                PtSize::Twelve => match cdb12(tf, data_in, ck_cond) {
                    Some(c) => self.t.execute(&c, data, timeout)?,
                    None => {
                        bail!("bridge only supports PASS-THROUGH(12); 48-bit command unavailable")
                    }
                },
            };
            let sense = scsi::parse_sense(&out.sense);
            let rejected = out.status == ScsiStatus::CheckCondition
                && sense.as_ref().is_some_and(|s| {
                    s.key == scsi::KEY_ILLEGAL_REQUEST
                        && s.ata.is_none()
                        && matches!(s.asc, 0x20 | 0x24)
                });
            if self.size == PtSize::Sixteen
                && !self.pt16_ok
                && rejected
                && cdb12(tf, data_in, ck_cond).is_some()
            {
                self.size = PtSize::Twelve;
                continue;
            }
            if self.size == PtSize::Sixteen && !rejected {
                self.pt16_ok = true;
            }
            return Ok((out.status, out.transferred, sense));
        }
    }

    /// Runs a PIO data-in command and returns its sectors.
    pub fn read(&mut self, cmd: AtaCmd) -> Result<Vec<u8>> {
        let len = cmd.data_len();
        assert!(len > 0, "read() needs a data-in command");
        let mut buf = vec![0u8; len];
        let (status, n, sense) = self.exec(&cmd.taskfile(), &mut buf, false)?;
        if status == ScsiStatus::CheckCondition {
            match sense {
                Some(s) if s.ata.is_some_and(|a| a.is_error()) => {
                    return Err(SatError::AtaAborted(s.ata.unwrap_or_default()).into());
                }
                Some(s)
                    if matches!(s.key, scsi::KEY_NO_SENSE | scsi::KEY_RECOVERED_ERROR)
                        && n == len => {}
                Some(s) => return Err(SatError::Rejected(s).into()),
                None => bail!("CHECK CONDITION without sense data"),
            }
        }
        if n < len {
            return Err(SatError::ShortRead(n, len).into());
        }
        Ok(buf)
    }

    /// Runs a non-data command with CK_COND=1 and returns the ATA registers.
    /// Fails with `SatError::AtaAborted` if the ATA status has ERR set.
    pub fn non_data(&mut self, cmd: AtaCmd) -> Result<Option<AtaReturn>> {
        let checked = |a: AtaReturn| -> Result<Option<AtaReturn>> {
            if a.is_error() {
                return Err(SatError::AtaAborted(a).into());
            }
            Ok(Some(a))
        };
        let (status, _, sense) = self.exec(&cmd.taskfile(), &mut [], true)?;
        if let Some(a) = sense.as_ref().and_then(|s| s.ata) {
            return checked(a);
        }
        if status == ScsiStatus::CheckCondition {
            return match sense {
                Some(s) => Err(SatError::Rejected(s).into()),
                None => bail!("CHECK CONDITION without sense data"),
            };
        }
        // Some bridges report GOOD and leave the registers for REQUEST SENSE.
        let raw = scsi::request_sense(&mut self.t, TIMEOUT)?;
        match scsi::parse_sense(&raw).and_then(|s| s.ata) {
            Some(a) => checked(a),
            None => Ok(None),
        }
    }

    /// Runs a non-data command without CK_COND; only success or failure matters.
    /// Some bridges return the ATA registers as recovered-error sense even on
    /// success; that counts as success unless the ATA status has ERR set.
    pub fn non_data_plain(&mut self, cmd: AtaCmd) -> Result<()> {
        self.non_data_plain_within(cmd, TIMEOUT)
    }

    /// As `non_data_plain` with an explicit CSW timeout.
    pub fn non_data_plain_within(&mut self, cmd: AtaCmd, timeout: Duration) -> Result<()> {
        let (status, _, sense) = self.exec_within(&cmd.taskfile(), &mut [], false, timeout)?;
        if status != ScsiStatus::CheckCondition {
            return Ok(());
        }
        let Some(s) = sense else {
            bail!("CHECK CONDITION without sense data");
        };
        match s.ata {
            Some(a) if a.is_error() => Err(SatError::AtaAborted(a).into()),
            Some(_) if matches!(s.key, scsi::KEY_NO_SENSE | scsi::KEY_RECOVERED_ERROR) => Ok(()),
            _ => Err(SatError::Rejected(s).into()),
        }
    }

    pub fn inquiry(&mut self) -> Result<scsi::Inquiry> {
        let mut buf = [0u8; 36];
        let out = self.t.execute(&scsi::inquiry_cdb(36), &mut buf, TIMEOUT)?;
        if out.status != ScsiStatus::Good {
            bail!("INQUIRY failed");
        }
        scsi::parse_inquiry(&buf[..out.transferred])
            .ok_or_else(|| anyhow::anyhow!("short INQUIRY data"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identify_cdb16() {
        let c = cdb16(&AtaCmd::Identify.taskfile(), true, false);
        assert_eq!(
            c,
            [0x85, 0x08, 0x0e, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0xec, 0]
        );
    }

    #[test]
    fn smart_return_status_cdb16() {
        let c = cdb16(&AtaCmd::SmartReturnStatus.taskfile(), false, true);
        assert_eq!(
            c,
            [
                0x85, 0x06, 0x20, 0, 0xda, 0, 0, 0, 0, 0, 0x4f, 0, 0xc2, 0, 0xb0, 0
            ]
        );
    }

    #[test]
    fn smart_read_log_cdb12() {
        let c = cdb12(&AtaCmd::SmartReadLog(0x06, 1).taskfile(), true, false).unwrap();
        assert_eq!(
            c,
            [0xa1, 0x08, 0x0e, 0xd5, 1, 0x06, 0x4f, 0xc2, 0, 0xb0, 0, 0]
        );
    }

    #[test]
    fn read_log_ext_cdb16() {
        let c = cdb16(&AtaCmd::ReadLogExt(0x04, 0x0102, 2).taskfile(), true, false);
        assert_eq!(c[1], 0x09, "PIO in + EXTEND");
        assert_eq!((c[5], c[6]), (0, 2));
        assert_eq!(c[8], 0x04, "log address in LBA(7:0)");
        assert_eq!(c[10], 0x02, "page low in LBA(15:8)");
        assert_eq!(c[11], 0x01, "page high in LBA(47:40)");
        assert_eq!(c[14], 0x2f);
        assert!(cdb12(&AtaCmd::ReadLogExt(0x04, 0x0102, 2).taskfile(), true, false).is_none());
        assert!(cdb12(&AtaCmd::ReadLogExt(0x07, 0, 1).taskfile(), true, false).is_some());
    }

    #[test]
    fn data_lengths() {
        assert_eq!(AtaCmd::Identify.data_len(), 512);
        assert_eq!(AtaCmd::SmartReadLog(0x04, 3).data_len(), 1536);
        assert_eq!(AtaCmd::SmartReturnStatus.data_len(), 0);
    }

    fn return_status(drive: crate::native::testutil::MockDrive) -> Result<Option<AtaReturn>> {
        Sat::new(drive).non_data(AtaCmd::SmartReturnStatus)
    }

    #[test]
    fn return_status_rejects_ata_error() {
        use crate::native::testutil::MockDrive;
        let ok = return_status(MockDrive::default()).unwrap().unwrap();
        assert_eq!((ok.lba_mid(), ok.lba_high()), (0x4f, 0xc2));

        let aborted = MockDrive {
            status_aborted: true,
            ..Default::default()
        };
        let e = return_status(aborted).unwrap_err();
        assert!(matches!(
            e.downcast_ref::<SatError>(),
            Some(SatError::AtaAborted(_))
        ));

        let via_rs = MockDrive {
            status_via_request_sense: true,
            ..Default::default()
        };
        assert_eq!(return_status(via_rs).unwrap().unwrap().lba_mid(), 0x4f);

        let via_rs_aborted = MockDrive {
            status_via_request_sense: true,
            status_aborted: true,
            ..Default::default()
        };
        assert!(return_status(via_rs_aborted).is_err());
    }
}
