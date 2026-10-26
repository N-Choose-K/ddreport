//! SCSI transport abstraction, INQUIRY, REQUEST SENSE and sense-data parsing.

use std::time::Duration;

use anyhow::Result;

pub const INQUIRY: u8 = 0x12;
pub const REQUEST_SENSE: u8 = 0x03;
pub const SENSE_ALLOC: u8 = 64;

pub const KEY_NO_SENSE: u8 = 0x00;
pub const KEY_RECOVERED_ERROR: u8 = 0x01;
pub const KEY_ILLEGAL_REQUEST: u8 = 0x05;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScsiStatus {
    Good,
    CheckCondition,
}

#[derive(Debug, Clone)]
pub struct ScsiOutcome {
    pub status: ScsiStatus,
    /// Bytes received in the data-in phase.
    pub transferred: usize,
    /// Sense data (autosense after CHECK CONDITION), empty if none.
    pub sense: Vec<u8>,
}

/// Executes SCSI commands. Only data-in (or no-data) commands exist: the
/// transport has no way to send a data-out payload to the device.
pub trait ScsiTransport {
    fn execute(&mut self, cdb: &[u8], data_in: &mut [u8], timeout: Duration)
    -> Result<ScsiOutcome>;
}

/// ATA register values returned by a SAT bridge (ATA Status Return descriptor
/// or fixed-format sense with "ATA PASS-THROUGH INFORMATION AVAILABLE").
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AtaReturn {
    pub extend: bool,
    pub error: u8,
    pub status: u8,
    pub device: u8,
    pub count: u16,
    pub lba: u64,
}

impl AtaReturn {
    pub fn lba_mid(&self) -> u8 {
        (self.lba >> 8) as u8
    }
    pub fn lba_high(&self) -> u8 {
        (self.lba >> 16) as u8
    }
    /// ATA status ERR bit.
    pub fn is_error(&self) -> bool {
        self.status & 0x01 != 0
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sense {
    pub response_code: u8,
    pub key: u8,
    pub asc: u8,
    pub ascq: u8,
    pub ata: Option<AtaReturn>,
}

impl std::fmt::Display for Sense {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "sense key 0x{:x}, ASC/ASCQ 0x{:02x}/0x{:02x}",
            self.key, self.asc, self.ascq
        )?;
        if let Some(a) = self.ata {
            write!(f, ", ATA status 0x{:02x} error 0x{:02x}", a.status, a.error)?;
        }
        Ok(())
    }
}

/// Parses fixed (0x70/0x71) or descriptor (0x72/0x73) format sense data.
pub fn parse_sense(b: &[u8]) -> Option<Sense> {
    let code = *b.first()? & 0x7f;
    match code {
        0x72 | 0x73 => {
            if b.len() < 4 {
                return None;
            }
            let mut s = Sense {
                response_code: code,
                key: b[1] & 0x0f,
                asc: b[2],
                ascq: b[3],
                ata: None,
            };
            let end = b.len().min(8 + *b.get(7).unwrap_or(&0) as usize);
            let mut i = 8;
            while i + 2 <= end {
                let (dcode, dlen) = (b[i], b[i + 1] as usize);
                let d = &b[i..(i + 2 + dlen).min(end)];
                if dcode == 0x09 && d.len() >= 14 {
                    let extend = d[2] & 0x01 != 0;
                    let mut lba = d[7] as u64 | (d[9] as u64) << 8 | (d[11] as u64) << 16;
                    let mut count = d[5] as u16;
                    if extend {
                        lba |= (d[6] as u64) << 24 | (d[8] as u64) << 32 | (d[10] as u64) << 40;
                        count |= (d[4] as u16) << 8;
                    }
                    s.ata = Some(AtaReturn {
                        extend,
                        error: d[3],
                        status: d[13],
                        device: d[12],
                        count,
                        lba,
                    });
                }
                i += 2 + dlen;
            }
            Some(s)
        }
        0x70 | 0x71 => {
            if b.len() < 14 {
                return None;
            }
            let mut s = Sense {
                response_code: code,
                key: b[2] & 0x0f,
                asc: b[12],
                ascq: b[13],
                ata: None,
            };
            if s.asc == 0x00 && s.ascq == 0x1d {
                s.ata = Some(AtaReturn {
                    extend: b[8] & 0x80 != 0,
                    error: b[3],
                    status: b[4],
                    device: b[5],
                    count: b[6] as u16,
                    lba: b[9] as u64 | (b[10] as u64) << 8 | (b[11] as u64) << 16,
                });
            }
            Some(s)
        }
        _ => None,
    }
}

pub fn inquiry_cdb(alloc: u8) -> [u8; 6] {
    [INQUIRY, 0, 0, 0, alloc, 0]
}

pub fn request_sense_cdb() -> [u8; 6] {
    [REQUEST_SENSE, 0, 0, 0, SENSE_ALLOC, 0]
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inquiry {
    pub peripheral_type: u8,
    pub vendor: String,
    pub product: String,
    pub revision: String,
}

impl std::fmt::Display for Inquiry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {} {}", self.vendor, self.product, self.revision)
    }
}

pub fn parse_inquiry(b: &[u8]) -> Option<Inquiry> {
    if b.len() < 36 {
        return None;
    }
    let s = |r: std::ops::Range<usize>| String::from_utf8_lossy(&b[r]).trim().to_owned();
    Some(Inquiry {
        peripheral_type: b[0] & 0x1f,
        vendor: s(8..16),
        product: s(16..32),
        revision: s(32..36),
    })
}

/// Issues REQUEST SENSE and returns the raw sense bytes.
pub fn request_sense<T: ScsiTransport + ?Sized>(t: &mut T, timeout: Duration) -> Result<Vec<u8>> {
    let mut buf = [0u8; SENSE_ALLOC as usize];
    let out = t.execute(&request_sense_cdb(), &mut buf, timeout)?;
    Ok(buf[..out.transferred].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::testutil::ata_desc_sense;

    #[test]
    fn descriptor_sense_ok() {
        let s = parse_sense(&ata_desc_sense(0x4f, 0xc2, 0x50)).unwrap();
        assert_eq!(s.key, KEY_RECOVERED_ERROR);
        let a = s.ata.unwrap();
        assert_eq!((a.lba_mid(), a.lba_high(), a.status), (0x4f, 0xc2, 0x50));
        assert!(!a.is_error());
    }

    #[test]
    fn descriptor_sense_extended() {
        let mut s = vec![0x72, 0x0b, 0, 0, 0, 0, 0, 14];
        s.extend_from_slice(&[
            0x09, 0x0c, 1, 0x04, 0x01, 0x02, 0xaa, 0x11, 0xbb, 0x22, 0xcc, 0x33, 0x40, 0x51,
        ]);
        let a = parse_sense(&s).unwrap().ata.unwrap();
        assert_eq!(a.lba, 0xccbb_aa33_2211);
        assert_eq!(a.count, 0x0102);
        assert_eq!(a.error, 0x04);
        assert!(a.is_error());
    }

    #[test]
    fn fixed_sense_with_ata_info() {
        let mut s = vec![0u8; 18];
        s[0] = 0x70;
        s[2] = KEY_RECOVERED_ERROR;
        s[3] = 0x00; // error
        s[4] = 0x50; // status
        s[10] = 0xf4;
        s[11] = 0x2c;
        s[12] = 0x00;
        s[13] = 0x1d;
        let a = parse_sense(&s).unwrap().ata.unwrap();
        assert_eq!((a.lba_mid(), a.lba_high()), (0xf4, 0x2c));
    }

    #[test]
    fn fixed_sense_illegal_request() {
        let mut s = vec![0u8; 18];
        s[0] = 0xf0;
        s[2] = KEY_ILLEGAL_REQUEST;
        s[12] = 0x20;
        let p = parse_sense(&s).unwrap();
        assert_eq!((p.key, p.asc, p.ata), (KEY_ILLEGAL_REQUEST, 0x20, None));
        assert!(parse_sense(&[0x00, 1, 2]).is_none());
        assert!(parse_sense(&[]).is_none());
    }

    #[test]
    fn inquiry() {
        let mut b = vec![0u8; 36];
        b[8..16].copy_from_slice(b"Innostor");
        b[16..32].copy_from_slice(b"Ext. HDD        ");
        b[32..36].copy_from_slice(b"1.00");
        let i = parse_inquiry(&b).unwrap();
        assert_eq!(i.to_string(), "Innostor Ext. HDD 1.00");
        assert!(parse_inquiry(&b[..20]).is_none());
    }
}
