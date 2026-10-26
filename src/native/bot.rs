//! USB Mass Storage Bulk-Only Transport: CBW/CSW framing, autosense and
//! reset recovery (BOT spec rev 1.0, sections 5 and 6).

use std::time::Duration;

use anyhow::{Result, bail};

use super::scsi::{self, ScsiOutcome, ScsiStatus, ScsiTransport};

pub const CBW_SIGNATURE: u32 = 0x4342_5355;
pub const CSW_SIGNATURE: u32 = 0x5342_5355;
pub const CBW_LEN: usize = 31;
pub const CSW_LEN: usize = 13;

const CSW_PASSED: u8 = 0;
const CSW_FAILED: u8 = 1;
const CSW_PHASE_ERROR: u8 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipeError {
    Stall,
    Timeout,
    Other(String),
}

impl std::fmt::Display for PipeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PipeError::Stall => f.write_str("endpoint stalled"),
            PipeError::Timeout => f.write_str("timed out"),
            PipeError::Other(s) => f.write_str(s),
        }
    }
}

impl std::error::Error for PipeError {}

/// Bulk endpoints plus the BOT class/standard requests used for recovery.
pub trait BulkPipe {
    fn write_out(&mut self, data: &[u8], timeout: Duration) -> Result<usize, PipeError>;
    fn read_in(&mut self, buf: &mut [u8], timeout: Duration) -> Result<usize, PipeError>;
    /// CLEAR_FEATURE(ENDPOINT_HALT) on the bulk-in endpoint.
    fn clear_halt_in(&mut self) -> Result<(), PipeError>;
    /// CLEAR_FEATURE(ENDPOINT_HALT) on the bulk-out endpoint.
    fn clear_halt_out(&mut self) -> Result<(), PipeError>;
    /// Bulk-Only Mass Storage Reset class request (0xFF).
    fn bot_reset(&mut self) -> Result<(), PipeError>;
}

pub fn encode_cbw(tag: u32, data_len: u32, data_in: bool, lun: u8, cdb: &[u8]) -> [u8; CBW_LEN] {
    assert!((1..=16).contains(&cdb.len()), "CDB length must be 1..=16");
    let mut b = [0u8; CBW_LEN];
    b[0..4].copy_from_slice(&CBW_SIGNATURE.to_le_bytes());
    b[4..8].copy_from_slice(&tag.to_le_bytes());
    b[8..12].copy_from_slice(&data_len.to_le_bytes());
    b[12] = if data_in && data_len > 0 { 0x80 } else { 0 };
    b[13] = lun & 0x0f;
    b[14] = cdb.len() as u8;
    b[15..15 + cdb.len()].copy_from_slice(cdb);
    b
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Csw {
    pub tag: u32,
    pub residue: u32,
    pub status: u8,
}

/// Decodes a CSW; None if the length or signature is wrong.
pub fn decode_csw(b: &[u8]) -> Option<Csw> {
    if b.len() != CSW_LEN || u32::from_le_bytes(b[0..4].try_into().ok()?) != CSW_SIGNATURE {
        return None;
    }
    Some(Csw {
        tag: u32::from_le_bytes(b[4..8].try_into().ok()?),
        residue: u32::from_le_bytes(b[8..12].try_into().ok()?),
        status: b[12],
    })
}

pub fn encode_csw(c: &Csw) -> [u8; CSW_LEN] {
    let mut b = [0u8; CSW_LEN];
    b[0..4].copy_from_slice(&CSW_SIGNATURE.to_le_bytes());
    b[4..8].copy_from_slice(&c.tag.to_le_bytes());
    b[8..12].copy_from_slice(&c.residue.to_le_bytes());
    b[12] = c.status;
    b
}

/// Validates the CSW residue against the requested and received lengths.
/// A passed command must satisfy received == requested - residue; a failed
/// command only needs a residue within the requested length (its data is
/// discarded, and many bridges report residue loosely on failure).
pub fn check_residue(
    status: u8,
    requested: usize,
    received: usize,
    residue: u32,
) -> Result<(), String> {
    let residue = residue as usize;
    if residue > requested {
        return Err(format!(
            "CSW residue {residue} exceeds requested length {requested}"
        ));
    }
    if status == CSW_PASSED && received != requested - residue {
        return Err(format!(
            "CSW residue {residue} disagrees with transfer: requested {requested}, received {received}"
        ));
    }
    Ok(())
}

/// Transport state after a failure: one retry after reset recovery, then every
/// command is skipped, so a wedged bridge costs a single timeout.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Health {
    Ok,
    /// Reset recovery ran after this failure; the next command is a probe.
    Recovered(String),
    /// The probe failed too; holds the first failure.
    Dead(String),
}

/// BOT transport over a `BulkPipe`.
pub struct Bot<P: BulkPipe> {
    pub pipe: P,
    pub lun: u8,
    tag: u32,
    health: Health,
}

impl<P: BulkPipe> Bot<P> {
    pub fn new(pipe: P, lun: u8) -> Self {
        Bot {
            pipe,
            lun,
            tag: 0,
            health: Health::Ok,
        }
    }

    /// Reset recovery: BOT reset, then clear halt on both bulk endpoints.
    fn reset_recovery(&mut self) {
        let _ = self.pipe.bot_reset();
        let _ = self.pipe.clear_halt_in();
        let _ = self.pipe.clear_halt_out();
    }

    fn fail<T>(&mut self, msg: String) -> Result<T> {
        match &self.health {
            Health::Ok => {
                self.reset_recovery();
                self.health = Health::Recovered(msg.clone());
            }
            Health::Recovered(first) => self.health = Health::Dead(first.clone()),
            Health::Dead(_) => {}
        }
        bail!("USB BOT: {msg}")
    }

    fn read_csw(&mut self, timeout: Duration) -> Result<Csw> {
        let mut buf = [0u8; CSW_LEN];
        for attempt in 0..2 {
            match self.pipe.read_in(&mut buf, timeout) {
                Ok(n) => match decode_csw(&buf[..n]) {
                    Some(c) => return Ok(c),
                    None => return self.fail(format!("invalid CSW ({n} bytes)")),
                },
                Err(PipeError::Stall) if attempt == 0 => {
                    if let Err(e) = self.pipe.clear_halt_in() {
                        return self.fail(format!("clear halt before CSW: {e}"));
                    }
                }
                Err(e) => return self.fail(format!("reading CSW: {e}")),
            }
        }
        self.fail("CSW endpoint stalled twice".into())
    }

    /// One CBW/data/CSW exchange. Returns (CSW status, bytes received).
    fn exchange(&mut self, cdb: &[u8], data: &mut [u8], timeout: Duration) -> Result<(u8, usize)> {
        if let Health::Dead(first) = &self.health {
            bail!("USB BOT: skipped, the bridge stopped responding ({first})");
        }
        self.tag = self.tag.wrapping_add(1);
        let tag = self.tag;
        let cbw = encode_cbw(tag, data.len() as u32, true, self.lun, cdb);
        match self.pipe.write_out(&cbw, timeout) {
            Ok(CBW_LEN) => {}
            Ok(n) => return self.fail(format!("short CBW write ({n} bytes)")),
            Err(e) => return self.fail(format!("sending CBW: {e}")),
        }

        let mut got = 0;
        let mut early_csw = None;
        if !data.is_empty() {
            match self.pipe.read_in(data, timeout) {
                Ok(n) => {
                    got = n;
                    // Some bridges skip the data phase and send the CSW directly.
                    if n == CSW_LEN
                        && data.len() > CSW_LEN
                        && let Some(c) = decode_csw(&data[..n]).filter(|c| c.tag == tag)
                    {
                        early_csw = Some(c);
                        got = 0;
                    }
                }
                Err(PipeError::Stall) => {
                    if let Err(e) = self.pipe.clear_halt_in() {
                        return self.fail(format!("clear halt after data stall: {e}"));
                    }
                }
                Err(e) => return self.fail(format!("data phase: {e}")),
            }
        }

        let csw = match early_csw {
            Some(c) => c,
            None => self.read_csw(timeout)?,
        };
        if csw.tag != tag {
            return self.fail(format!("CSW tag mismatch (sent {tag}, got {})", csw.tag));
        }
        match csw.status {
            CSW_PASSED | CSW_FAILED => {}
            CSW_PHASE_ERROR => return self.fail("phase error".into()),
            s => return self.fail(format!("invalid CSW status {s}")),
        }
        match check_residue(csw.status, data.len(), got, csw.residue) {
            Ok(()) => {
                self.health = Health::Ok;
                Ok((csw.status, got))
            }
            Err(msg) => self.fail(msg),
        }
    }
}

impl<P: BulkPipe> ScsiTransport for Bot<P> {
    fn execute(&mut self, cdb: &[u8], data: &mut [u8], timeout: Duration) -> Result<ScsiOutcome> {
        let (status, transferred) = self.exchange(cdb, data, timeout)?;
        if status == CSW_PASSED {
            return Ok(ScsiOutcome {
                status: ScsiStatus::Good,
                transferred,
                sense: Vec::new(),
            });
        }
        let sense = if cdb[0] == scsi::REQUEST_SENSE {
            Vec::new()
        } else {
            let mut buf = [0u8; scsi::SENSE_ALLOC as usize];
            match self.exchange(&scsi::request_sense_cdb(), &mut buf, timeout)? {
                (CSW_PASSED, n) => buf[..n].to_vec(),
                _ => Vec::new(),
            }
        };
        Ok(ScsiOutcome {
            status: ScsiStatus::CheckCondition,
            transferred,
            sense,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// Scripted pipe: each IN read pops the next response.
    #[derive(Default)]
    struct Script {
        out: Vec<Vec<u8>>,
        ins: VecDeque<Result<Vec<u8>, PipeError>>,
        resets: usize,
        clear_in: usize,
        clear_out: usize,
    }

    impl BulkPipe for &mut Script {
        fn write_out(&mut self, data: &[u8], _: Duration) -> Result<usize, PipeError> {
            self.out.push(data.to_vec());
            Ok(data.len())
        }
        fn read_in(&mut self, buf: &mut [u8], _: Duration) -> Result<usize, PipeError> {
            let r = self.ins.pop_front().unwrap_or(Err(PipeError::Timeout))?;
            let n = r.len().min(buf.len());
            buf[..n].copy_from_slice(&r[..n]);
            Ok(n)
        }
        fn clear_halt_in(&mut self) -> Result<(), PipeError> {
            self.clear_in += 1;
            Ok(())
        }
        fn clear_halt_out(&mut self) -> Result<(), PipeError> {
            self.clear_out += 1;
            Ok(())
        }
        fn bot_reset(&mut self) -> Result<(), PipeError> {
            self.resets += 1;
            Ok(())
        }
    }

    fn csw(tag: u32, residue: u32, status: u8) -> Result<Vec<u8>, PipeError> {
        Ok(encode_csw(&Csw {
            tag,
            residue,
            status,
        })
        .to_vec())
    }

    const T: Duration = Duration::from_secs(1);

    #[test]
    fn cbw_layout() {
        let b = encode_cbw(0x1234_5678, 512, true, 0, &[0x85, 1, 2]);
        assert_eq!(&b[0..4], b"USBC");
        assert_eq!(&b[4..8], &[0x78, 0x56, 0x34, 0x12]);
        assert_eq!(&b[8..12], &[0, 2, 0, 0]);
        assert_eq!((b[12], b[13], b[14]), (0x80, 0, 3));
        assert_eq!(&b[15..18], &[0x85, 1, 2]);
        assert!(b[18..].iter().all(|&x| x == 0));
        assert_eq!(
            encode_cbw(1, 0, true, 0, &[0])[12],
            0,
            "no direction without data"
        );
    }

    #[test]
    fn csw_roundtrip_and_rejects() {
        let c = Csw {
            tag: 7,
            residue: 3,
            status: 1,
        };
        let b = encode_csw(&c);
        assert_eq!(&b[0..4], b"USBS");
        assert_eq!(decode_csw(&b), Some(c));
        assert_eq!(decode_csw(&b[..12]), None);
        let mut bad = b;
        bad[0] = b'X';
        assert_eq!(decode_csw(&bad), None);
    }

    #[test]
    fn wedged_bridge_gets_one_probe_then_skips() {
        let mut s = Script::default();
        let mut bot = Bot::new(&mut s, 0);
        let cdb = [0x12, 0, 0, 0, 0, 0];
        let e = bot.execute(&cdb, &mut [], T).unwrap_err().to_string();
        assert!(e.contains("reading CSW"), "{e}");
        let e = bot.execute(&cdb, &mut [], T).unwrap_err().to_string();
        assert!(e.contains("reading CSW"), "{e}");
        let e = bot.execute(&cdb, &mut [], T).unwrap_err().to_string();
        assert!(e.contains("skipped, the bridge stopped responding"), "{e}");
        assert_eq!(
            s.out.len(),
            2,
            "the probe was sent; later commands were not"
        );
        assert_eq!(s.resets, 1, "one reset recovery, before the probe");
    }

    #[test]
    fn probe_success_clears_the_failure() {
        let mut s = Script::default();
        s.ins.push_back(Err(PipeError::Timeout));
        s.ins.push_back(csw(2, 0, 0));
        s.ins.push_back(csw(3, 0, 0));
        let mut bot = Bot::new(&mut s, 0);
        let cdb = [0x12, 0, 0, 0, 0, 0];
        assert!(bot.execute(&cdb, &mut [], T).is_err());
        assert!(bot.execute(&cdb, &mut [], T).is_ok());
        assert!(bot.execute(&cdb, &mut [], T).is_ok());
        assert_eq!((s.out.len(), s.resets), (3, 1));
    }

    #[test]
    fn good_data_in() {
        let mut s = Script::default();
        s.ins.push_back(Ok(vec![0xab; 36]));
        s.ins.push_back(csw(1, 0, 0));
        let mut bot = Bot::new(&mut s, 0);
        let mut buf = [0u8; 36];
        let out = bot.execute(&[0x12, 0, 0, 0, 36, 0], &mut buf, T).unwrap();
        assert_eq!((out.status, out.transferred), (ScsiStatus::Good, 36));
        assert_eq!(buf[0], 0xab);
        assert_eq!(s.out.len(), 1);
        assert_eq!(s.resets, 0);
    }

    #[test]
    fn failed_command_fetches_sense() {
        let mut s = Script::default();
        s.ins.push_back(csw(1, 0, 1)); // no-data command fails
        s.ins
            .push_back(Ok(vec![0x70, 0, 5, 0, 0, 0, 0, 10, 0, 0, 0, 0, 0x20, 0]));
        s.ins.push_back(csw(2, 50, 0));
        let mut bot = Bot::new(&mut s, 0);
        let out = bot.execute(&[0x85; 16], &mut [], T).unwrap();
        assert_eq!(out.status, ScsiStatus::CheckCondition);
        assert_eq!(out.sense.len(), 14);
        assert_eq!(s.out[1][15], scsi::REQUEST_SENSE);
    }

    #[test]
    fn data_stall_clears_halt_then_reads_csw() {
        let mut s = Script::default();
        s.ins.push_back(Err(PipeError::Stall));
        s.ins.push_back(csw(1, 512, 1));
        s.ins
            .push_back(Ok(vec![0x70, 0, 5, 0, 0, 0, 0, 10, 0, 0, 0, 0, 0x24, 0]));
        s.ins.push_back(csw(2, 50, 0));
        let mut bot = Bot::new(&mut s, 0);
        let out = bot.execute(&[0x85; 16], &mut [0u8; 512], T).unwrap();
        assert_eq!(
            (out.status, out.transferred),
            (ScsiStatus::CheckCondition, 0)
        );
        assert_eq!(s.clear_in, 1);
        assert_eq!(s.resets, 0);
    }

    #[test]
    fn csw_stall_retries_once() {
        let mut s = Script::default();
        s.ins.push_back(Err(PipeError::Stall));
        s.ins.push_back(csw(1, 0, 0));
        let mut bot = Bot::new(&mut s, 0);
        assert!(bot.execute(&[0x00; 6], &mut [], T).is_ok());
        assert_eq!(s.clear_in, 1);
    }

    #[test]
    fn tag_mismatch_resets() {
        let mut s = Script::default();
        s.ins.push_back(csw(99, 0, 0));
        let mut bot = Bot::new(&mut s, 0);
        let e = bot.execute(&[0x00; 6], &mut [], T).unwrap_err();
        assert!(e.to_string().contains("tag mismatch"));
        assert_eq!((s.resets, s.clear_in, s.clear_out), (1, 1, 1));
    }

    #[test]
    fn phase_error_resets() {
        let mut s = Script::default();
        s.ins.push_back(csw(1, 0, 2));
        let mut bot = Bot::new(&mut s, 0);
        assert!(bot.execute(&[0x00; 6], &mut [], T).is_err());
        assert_eq!(s.resets, 1);
    }

    #[test]
    fn early_csw_in_data_phase() {
        let mut s = Script::default();
        s.ins.push_back(csw(1, 512, 1));
        s.ins
            .push_back(Ok(vec![0x70, 0, 5, 0, 0, 0, 0, 10, 0, 0, 0, 0, 0x20, 0]));
        s.ins.push_back(csw(2, 50, 0));
        let mut bot = Bot::new(&mut s, 0);
        let out = bot.execute(&[0x85; 16], &mut [0u8; 512], T).unwrap();
        assert_eq!(
            (out.status, out.transferred),
            (ScsiStatus::CheckCondition, 0)
        );
    }

    #[test]
    fn invalid_csw_resets() {
        let mut s = Script::default();
        s.ins.push_back(Ok(vec![0; 13]));
        let mut bot = Bot::new(&mut s, 0);
        assert!(bot.execute(&[0x00; 6], &mut [], T).is_err());
        assert_eq!(s.resets, 1);
    }

    #[test]
    fn residue_rules() {
        assert!(check_residue(CSW_PASSED, 512, 512, 0).is_ok());
        assert!(check_residue(CSW_PASSED, 512, 100, 412).is_ok());
        assert!(
            check_residue(CSW_PASSED, 512, 512, 12).is_err(),
            "full data but residue"
        );
        assert!(
            check_residue(CSW_PASSED, 512, 100, 0).is_err(),
            "short data, zero residue"
        );
        assert!(
            check_residue(CSW_PASSED, 0, 0, 1).is_err(),
            "residue on no-data command"
        );
        assert!(check_residue(CSW_FAILED, 512, 0, 0).is_ok());
        assert!(check_residue(CSW_FAILED, 512, 0, 513).is_err());
    }

    #[test]
    fn short_transfer_reported() {
        let mut s = Script::default();
        s.ins.push_back(Ok(vec![0xab; 100]));
        s.ins.push_back(csw(1, 412, 0));
        let mut bot = Bot::new(&mut s, 0);
        let out = bot.execute(&[0x85; 16], &mut [0u8; 512], T).unwrap();
        assert_eq!((out.status, out.transferred), (ScsiStatus::Good, 100));
    }

    #[test]
    fn residue_mismatch_resets() {
        let mut s = Script::default();
        s.ins.push_back(Ok(vec![0xab; 512]));
        s.ins.push_back(csw(1, 12, 0));
        let mut bot = Bot::new(&mut s, 0);
        let e = bot.execute(&[0x85; 16], &mut [0u8; 512], T).unwrap_err();
        assert!(e.to_string().contains("residue"));
        assert_eq!(s.resets, 1);
    }
}
