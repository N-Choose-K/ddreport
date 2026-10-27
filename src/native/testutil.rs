//! Synthetic ATA sectors and a mock SAT drive for unit tests.

use std::time::Duration;

use anyhow::Result;

use super::scsi::{self, ScsiOutcome, ScsiStatus, ScsiTransport};

fn put_word(b: &mut [u8], i: usize, v: u16) {
    b[2 * i..2 * i + 2].copy_from_slice(&v.to_le_bytes());
}

fn put_ata_string(b: &mut [u8], first: usize, len_words: usize, s: &str) {
    let mut padded = s.as_bytes().to_vec();
    padded.resize(len_words * 2, b' ');
    for i in 0..len_words {
        b[2 * (first + i)] = padded[2 * i + 1];
        b[2 * (first + i) + 1] = padded[2 * i];
    }
}

/// Sets byte 511 so the 512-byte structure checksums to zero.
pub fn fix_checksum(b: &mut [u8]) {
    let sum = b[..511].iter().fold(0u8, |a, &x| a.wrapping_add(x));
    b[511] = 0u8.wrapping_sub(sum);
}

pub struct IdentifySpec {
    pub model: &'static str,
    pub serial: &'static str,
    pub firmware: &'static str,
    pub sectors: u64,
    pub lba48: bool,
    pub physical_exp: u16,
    pub rotation: u16,
    pub form_factor: u16,
    pub trim: bool,
    pub gpl: bool,
}

impl Default for IdentifySpec {
    fn default() -> Self {
        IdentifySpec {
            model: "WDC WD80EFAX-68KNBN0",
            serial: "VAGABCDE",
            firmware: "81.00A81",
            sectors: 15_628_053_168,
            lba48: true,
            physical_exp: 3,
            rotation: 5400,
            form_factor: 2,
            trim: false,
            gpl: true,
        }
    }
}

pub fn identify_sector(s: &IdentifySpec) -> Vec<u8> {
    let mut b = vec![0u8; 512];
    put_ata_string(&mut b, 10, 10, s.serial);
    put_ata_string(&mut b, 23, 4, s.firmware);
    put_ata_string(&mut b, 27, 20, s.model);
    put_word(&mut b, 60, s.sectors.min(0x0fff_ffff) as u16);
    put_word(&mut b, 61, (s.sectors.min(0x0fff_ffff) >> 16) as u16);
    if s.trim {
        put_word(&mut b, 69, (1 << 14) | (1 << 5));
    }
    put_word(&mut b, 76, 0b1110);
    put_word(&mut b, 77, 3 << 1);
    put_word(&mut b, 80, 0x07f0);
    put_word(&mut b, 81, 0x006d);
    put_word(&mut b, 82, 0x0001);
    put_word(&mut b, 83, 0x4000 | if s.lba48 { 1 << 10 } else { 0 });
    let gpl = if s.gpl { 0x20 } else { 0 };
    put_word(&mut b, 84, 0x4003 | gpl);
    put_word(&mut b, 85, 0x0001);
    put_word(&mut b, 87, 0x4103 | gpl);
    if s.lba48 {
        for i in 0..4 {
            put_word(&mut b, 100 + i, (s.sectors >> (16 * i)) as u16);
        }
    }
    put_word(
        &mut b,
        106,
        0x4000
            | if s.physical_exp > 0 {
                0x2000 | s.physical_exp
            } else {
                0
            },
    );
    put_word(&mut b, 108, 0x5001);
    put_word(&mut b, 109, 0x4ee2);
    put_word(&mut b, 110, 0xb123);
    put_word(&mut b, 111, 0x4567);
    put_word(&mut b, 168, s.form_factor);
    put_word(&mut b, 169, s.trim as u16);
    put_word(&mut b, 206, 0x0001);
    put_word(&mut b, 217, s.rotation);
    put_word(&mut b, 222, 0x107f);
    b[510] = 0xa5;
    fix_checksum(&mut b);
    b
}

/// (id, flags, value, worst, thresh, raw)
pub type AttrSpec = (u8, u16, u8, u8, u8, u64);

pub fn sample_attrs() -> Vec<AttrSpec> {
    vec![
        (1, 0x002f, 100, 100, 51, 0),
        (5, 0x0033, 200, 200, 140, 0),
        (9, 0x0032, 90, 90, 0, 12_345),
        (12, 0x0032, 100, 100, 0, 42),
        (194, 0x0022, 118, 100, 0, 0x002d_0012_0022),
        (197, 0x0032, 200, 200, 0, 0),
        (199, 0x0032, 200, 200, 0, 3),
    ]
}

pub fn smart_data_sector(attrs: &[AttrSpec], exec_status: u8) -> Vec<u8> {
    let mut b = vec![0u8; 512];
    put_word(&mut b, 0, 0x0010);
    for (i, &(id, flags, value, worst, _, raw)) in attrs.iter().enumerate() {
        let e = &mut b[2 + 12 * i..14 + 12 * i];
        e[0] = id;
        e[1..3].copy_from_slice(&flags.to_le_bytes());
        e[3] = value;
        e[4] = worst;
        e[5..11].copy_from_slice(&raw.to_le_bytes()[..6]);
    }
    b[362] = 0x82;
    b[363] = exec_status;
    b[367] = 0x7b;
    b[368] = 0x03;
    b[370] = 0x01;
    b[372] = 2;
    b[373] = 0xff;
    b[374] = 5;
    b[375..377].copy_from_slice(&1000u16.to_le_bytes());
    fix_checksum(&mut b);
    b
}

pub fn thresholds_sector(attrs: &[AttrSpec]) -> Vec<u8> {
    let mut b = vec![0u8; 512];
    put_word(&mut b, 0, 0x0010);
    for (i, &(id, _, _, _, thresh, _)) in attrs.iter().enumerate() {
        b[2 + 12 * i] = id;
        b[3 + 12 * i] = thresh;
    }
    fix_checksum(&mut b);
    b
}

/// (1-based slot, type, status byte, hours, lba)
pub type SelfTestSpec = (usize, u8, u8, u16, u64);

pub fn selftest_log_sector(entries: &[SelfTestSpec], index: u8) -> Vec<u8> {
    let mut b = vec![0u8; 512];
    put_word(&mut b, 0, 1);
    for &(slot, kind, status, hours, lba) in entries {
        let d = &mut b[2 + 24 * (slot - 1)..26 + 24 * (slot - 1)];
        d[0] = kind;
        d[1] = status;
        d[2..4].copy_from_slice(&hours.to_le_bytes());
        d[5..9].copy_from_slice(&(lba as u32).to_le_bytes());
    }
    b[508] = index;
    fix_checksum(&mut b);
    b
}

/// Multi-page extended self-test log; slots are 1-based across all pages.
pub fn ext_selftest_log_pages(pages: usize, entries: &[SelfTestSpec], index: u16) -> Vec<u8> {
    let mut b = vec![0u8; 512 * pages];
    b[0] = 1;
    put_word(&mut b, 1, index);
    for &(slot, kind, status, hours, lba) in entries {
        let (page, i) = ((slot - 1) / 19, (slot - 1) % 19);
        let o = 512 * page + 4 + 26 * i;
        let d = &mut b[o..o + 26];
        d[0] = kind;
        d[1] = status;
        d[2..4].copy_from_slice(&hours.to_le_bytes());
        d[5..11].copy_from_slice(&lba.to_le_bytes()[..6]);
    }
    b
}

pub fn devstat_page0(pages: &[u8]) -> Vec<u8> {
    let mut b = vec![0u8; 512];
    b[0] = 1;
    b[8] = pages.len() as u8 + 1;
    b[9] = 0;
    b[10..10 + pages.len()].copy_from_slice(pages);
    b
}

/// Entries are (byte offset, flags byte, value).
pub fn devstat_page(page: u8, entries: &[(usize, u8, u64)]) -> Vec<u8> {
    let mut b = vec![0u8; 512];
    b[0..8].copy_from_slice(&(1u64 | (page as u64) << 16).to_le_bytes());
    for &(off, flags, value) in entries {
        b[off..off + 8].copy_from_slice(&(value | (flags as u64) << 56).to_le_bytes());
    }
    b
}

pub fn sct_status_sector(current: i8, lifetime_max: i8) -> Vec<u8> {
    let mut b = vec![0u8; 512];
    put_word(&mut b, 0, 3);
    put_word(&mut b, 1, 1);
    b[200] = current as u8;
    b[201] = 0x80;
    b[202] = 0x80;
    b[203] = 0x80;
    b[204] = lifetime_max as u8;
    b
}

pub fn log_directory_sector(logs: &[(u8, u16)]) -> Vec<u8> {
    let mut b = vec![0u8; 512];
    put_word(&mut b, 0, 1);
    for &(addr, pages) in logs {
        put_word(&mut b, addr as usize, pages);
    }
    b
}

/// Descriptor sense as returned for a CK_COND=1 command.
pub fn ata_desc_sense(lba_mid: u8, lba_high: u8, status: u8) -> Vec<u8> {
    let mut s = vec![0x72, scsi::KEY_RECOVERED_ERROR, 0x00, 0x1d, 0, 0, 0, 14];
    s.extend_from_slice(&[
        0x09, 0x0c, 0, 0, 0, 0, 0, 0, 0, lba_mid, 0, lba_high, 0, status,
    ]);
    s
}

fn illegal_request(asc: u8) -> Vec<u8> {
    let mut s = vec![0u8; 18];
    s[0] = 0x70;
    s[2] = scsi::KEY_ILLEGAL_REQUEST;
    s[7] = 10;
    s[12] = asc;
    s
}

fn ata_abort() -> Vec<u8> {
    let mut s = vec![0x72, 0x0b, 0, 0, 0, 0, 0, 14];
    s.extend_from_slice(&[0x09, 0x0c, 0, 0x04, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x51]);
    s
}

/// SAT drive simulator: decodes PASS-THROUGH CDBs and answers from sectors.
pub struct MockDrive {
    pub identify: Vec<u8>,
    pub smart_data: Vec<u8>,
    pub thresholds: Vec<u8>,
    pub failing: bool,
    pub selftest_log: Vec<u8>,
    pub ext_selftest_log: Vec<u8>,
    pub gp_dir: Vec<u8>,
    pub smart_dir: Vec<u8>,
    /// Device statistics pages, page N at index N.
    pub devstats: Vec<Vec<u8>>,
    pub sct: Vec<u8>,
    pub error_log: Vec<u8>,
    pub reject_pt16: bool,
    /// SMART RETURN STATUS sets ATA ERR/ABRT while echoing the signature.
    pub status_aborted: bool,
    /// SMART EXECUTE OFF-LINE IMMEDIATE is refused (drive aborts it).
    pub offline_rejected: bool,
    /// The bridge answers it with recovered-error sense carrying a clean status.
    pub offline_sense: bool,
    /// The bridge never answers it (CSW timeout).
    pub offline_no_csw: bool,
    /// SMART RETURN STATUS answers GOOD; registers come from REQUEST SENSE.
    pub status_via_request_sense: bool,
    pub pending_sense: Vec<u8>,
    /// Every CDB received, in order.
    pub cdbs: Vec<Vec<u8>>,
}

impl Default for MockDrive {
    fn default() -> Self {
        let attrs = sample_attrs();
        let mut error_log = vec![0u8; 512];
        error_log[0] = 1;
        MockDrive {
            identify: identify_sector(&IdentifySpec::default()),
            smart_data: smart_data_sector(&attrs, 0x00),
            thresholds: thresholds_sector(&attrs),
            failing: false,
            selftest_log: selftest_log_sector(
                &[(1, 0x01, 0x00, 12_000, 0), (2, 0x02, 0x00, 12_300, 0)],
                2,
            ),
            ext_selftest_log: ext_selftest_log_pages(
                1,
                &[(1, 0x01, 0x00, 12_000, 0), (2, 0x02, 0x00, 12_300, 0)],
                2,
            ),
            gp_dir: log_directory_sector(&[(0x04, 8), (0x07, 1), (0xe0, 1)]),
            smart_dir: log_directory_sector(&[(0x01, 1), (0x04, 8), (0x06, 1), (0xe0, 1)]),
            devstats: vec![
                devstat_page0(&[1, 3, 5]),
                devstat_page(1, &[(8, 0xc0, 77), (16, 0xc0, 12_345)]),
                Vec::new(),
                devstat_page(3, &[(32, 0xc0, 0)]),
                Vec::new(),
                devstat_page(5, &[(8, 0xc0, 34), (32, 0xc0, 52)]),
            ],
            sct: sct_status_sector(34, 52),
            error_log,
            reject_pt16: false,
            status_aborted: false,
            offline_rejected: false,
            offline_sense: false,
            offline_no_csw: false,
            status_via_request_sense: false,
            pending_sense: Vec::new(),
            cdbs: Vec::new(),
        }
    }
}

impl MockDrive {
    fn respond(&self, cdb: &[u8], data: &mut [u8]) -> (usize, Option<Vec<u8>>) {
        let regs = match cdb[0] {
            scsi::INQUIRY => {
                let mut inq = [b' '; 36];
                inq[0] = 0;
                inq[8..16].copy_from_slice(b"Innostor");
                inq[16..24].copy_from_slice(b"Ext. HDD");
                inq[32..36].copy_from_slice(b"1.00");
                let n = data.len().min(36);
                data[..n].copy_from_slice(&inq[..n]);
                return (n, None);
            }
            0x85 if self.reject_pt16 => return (0, Some(illegal_request(0x20))),
            // (features, count, lba_low, lba_mid, lba_high, command, page)
            0x85 => (
                cdb[4],
                cdb[6],
                cdb[8],
                cdb[10],
                cdb[12],
                cdb[14],
                cdb[10] as u16 | (cdb[11] as u16) << 8,
            ),
            0xa1 => (
                cdb[3],
                cdb[4],
                cdb[5],
                cdb[6],
                cdb[7],
                cdb[9],
                cdb[6] as u16,
            ),
            _ => return (0, Some(illegal_request(0x20))),
        };
        let (features, count, lba_low, lba_mid, lba_high, command, page) = regs;
        let ck_cond = cdb[2] & 0x20 != 0;
        let sectors: Option<Vec<u8>> = match (command, features) {
            (0xec, _) => Some(self.identify.clone()),
            (0xb0, _) if (lba_mid, lba_high) != (0x4f, 0xc2) => None,
            (0xb0, 0xd0) => Some(self.smart_data.clone()),
            (0xb0, 0xd1) => Some(self.thresholds.clone()),
            (0xb0, 0xda) if ck_cond => {
                let (m, h) = if self.failing {
                    (0xf4, 0x2c)
                } else {
                    (0x4f, 0xc2)
                };
                let status = if self.status_aborted { 0x51 } else { 0x50 };
                return (0, Some(ata_desc_sense(m, h, status)));
            }
            (0xb0, 0xd4) => {
                return if self.offline_rejected {
                    (0, Some(ata_desc_sense(0x4f, 0xc2, 0x51)))
                } else if self.offline_sense {
                    (0, Some(ata_desc_sense(0x4f, 0xc2, 0x50)))
                } else {
                    (0, None)
                };
            }
            (0xb0, 0xd5) => match lba_low {
                0x00 => Some(self.smart_dir.clone()),
                0x01 => Some(self.error_log.clone()),
                0x06 => Some(self.selftest_log.clone()),
                0xe0 => Some(self.sct.clone()),
                0x04 if !self.devstats.is_empty() => Some(
                    self.devstats
                        .iter()
                        .take(count as usize)
                        .flat_map(|p| {
                            let mut p = p.clone();
                            p.resize(512, 0);
                            p
                        })
                        .collect(),
                ),
                _ => None,
            },
            (0x2f, _) => match lba_low {
                0x00 => Some(self.gp_dir.clone()),
                0x07 => self
                    .ext_selftest_log
                    .get(512 * page as usize..512 * (page as usize + count as usize))
                    .map(<[u8]>::to_vec),
                0xe0 => Some(self.sct.clone()),
                0x04 => self
                    .devstats
                    .get(page as usize)
                    .filter(|p| !p.is_empty())
                    .cloned(),
                _ => None,
            },
            _ => None,
        };
        match sectors {
            Some(mut s) => {
                s.resize(data.len(), 0);
                data.copy_from_slice(&s);
                (data.len(), None)
            }
            None => (0, Some(ata_abort())),
        }
    }
}

impl ScsiTransport for MockDrive {
    fn execute(&mut self, cdb: &[u8], data: &mut [u8], _: Duration) -> Result<ScsiOutcome> {
        self.cdbs.push(cdb.to_vec());
        if cdb[0] == scsi::REQUEST_SENSE {
            let s = std::mem::take(&mut self.pending_sense);
            let n = s.len().min(data.len());
            data[..n].copy_from_slice(&s[..n]);
            return Ok(ScsiOutcome {
                status: ScsiStatus::Good,
                transferred: n,
                sense: Vec::new(),
            });
        }
        if self.offline_no_csw && cdb[0] == 0x85 && cdb[4] == 0xd4 {
            return Err(anyhow::Error::new(crate::native::bot::CswTimeout)
                .context("USB BOT: reading CSW: timed out"));
        }
        let (transferred, mut sense) = self.respond(cdb, data);
        let is_return_status = cdb[0] == 0x85 && cdb[4] == 0xda;
        if self.status_via_request_sense && is_return_status {
            self.pending_sense = sense.take().unwrap_or_default();
        }
        Ok(match sense {
            Some(sense) => ScsiOutcome {
                status: ScsiStatus::CheckCondition,
                transferred,
                sense,
            },
            None => ScsiOutcome {
                status: ScsiStatus::Good,
                transferred,
                sense: Vec::new(),
            },
        })
    }
}
