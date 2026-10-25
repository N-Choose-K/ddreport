//! Parsers for ATA IDENTIFY DEVICE, SMART data/thresholds, self-test logs,
//! device statistics, SCT status and log directories (ACS-3/ACS-4 layouts).

fn word(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[2 * i], b[2 * i + 1]])
}

fn qword(b: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(b[off..off + 8].try_into().expect("8 bytes"))
}

/// Word is "valid" in the ATA sense: not 0x0000 and not 0xFFFF.
fn valid(w: u16) -> bool {
    w != 0 && w != 0xffff
}

/// ATA string from words `first..=last`: each word holds two chars, high byte first.
pub fn ata_string(b: &[u8], first: usize, last: usize) -> String {
    let bytes: Vec<u8> = (first..=last)
        .flat_map(|i| [b[2 * i + 1], b[2 * i]])
        .collect();
    String::from_utf8_lossy(&bytes)
        .trim_matches(|c: char| c.is_whitespace() || c == '\0')
        .to_owned()
}

/// 512-byte structure checksum: all bytes sum to 0 mod 256.
pub fn checksum_ok(b: &[u8]) -> bool {
    b[..512].iter().fold(0u8, |a, &x| a.wrapping_add(x)) == 0
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Wwn {
    pub naa: u8,
    pub oui: u32,
    pub id: u64,
}

#[derive(Debug, Clone, Default)]
pub struct Identify {
    pub model: String,
    pub serial: String,
    pub firmware: String,
    pub sectors: u64,
    pub logical_sector: u32,
    pub physical_sector: u32,
    /// Word 217: 1 = non-rotating, >= 0x401 = RPM, None = not reported.
    pub rotation: Option<u16>,
    pub form_factor: Option<u8>,
    pub wwn: Option<Wwn>,
    pub ata_major: u16,
    pub ata_minor: u16,
    /// Word 76 (SATA capabilities), None if invalid.
    pub sata_caps: Option<u16>,
    /// Word 77 bits 3:1 (current negotiated speed), None if not reported.
    pub sata_current: Option<u8>,
    /// Word 222 (transport major version), None if not serial.
    pub sata_version: Option<u16>,
    pub trim: Option<bool>,
    pub trim_deterministic: bool,
    pub trim_zeroed: bool,
    pub smart_supported: bool,
    pub smart_enabled: bool,
    pub self_test_supported: bool,
    pub error_log_supported: bool,
    pub gpl_supported: bool,
    pub sct_supported: bool,
    /// None if the IDENTIFY data has no checksum signature (word 255 low byte != 0xA5).
    pub checksum_ok: Option<bool>,
}

pub fn parse_identify(b: &[u8]) -> Identify {
    let w = |i| word(b, i);
    let w83_ok = w(83) & 0xc000 == 0x4000;
    let w84_ok = w(84) & 0xc000 == 0x4000;
    let w87_ok = w(87) & 0xc000 == 0x4000;
    let lba48 = w83_ok && w(83) & (1 << 10) != 0;

    let mut sectors = if lba48 {
        (0..4).fold(0u64, |a, i| a | (w(100 + i) as u64) << (16 * i))
    } else {
        0
    };
    if w(69) & (1 << 3) != 0 {
        let ext = (0..4).fold(0u64, |a, i| a | (w(230 + i) as u64) << (16 * i));
        if ext != 0 {
            sectors = ext;
        }
    }
    if sectors == 0 {
        sectors = w(60) as u64 | (w(61) as u64) << 16;
    }

    let w106 = w(106);
    let w106_ok = w106 & 0xc000 == 0x4000;
    let mut logical = 512u32;
    if w106_ok && w106 & (1 << 12) != 0 {
        let words = w(117) as u32 | (w(118) as u32) << 16;
        if words >= 256 {
            logical = words * 2;
        }
    }
    let physical = if w106_ok && w106 & (1 << 13) != 0 {
        logical << (w106 & 0x0f)
    } else {
        logical
    };

    let wwn =
        (w87_ok && w(87) & (1 << 8) != 0 && (w(108) | w(109) | w(110) | w(111)) != 0).then(|| {
            Wwn {
                naa: (w(108) >> 12) as u8,
                oui: ((w(108) as u32 & 0x0fff) << 12) | (w(109) as u32 >> 4),
                id: ((w(109) as u64 & 0xf) << 32) | (w(110) as u64) << 16 | w(111) as u64,
            }
        });

    let rot = w(217);
    let ff = w(168) & 0x0f;
    let sata_caps = valid(w(76)).then_some(w(76));
    let cur = ((w(77) >> 1) & 0x7) as u8;
    let w222 = w(222);
    let trim = (w(169) != 0xffff).then_some(w(169) & 1 != 0);

    Identify {
        model: ata_string(b, 27, 46),
        serial: ata_string(b, 10, 19),
        firmware: ata_string(b, 23, 26),
        sectors,
        logical_sector: logical,
        physical_sector: physical,
        rotation: (rot == 1 || (0x0401..0xffff).contains(&rot)).then_some(rot),
        form_factor: (1..=9).contains(&ff).then_some(ff as u8),
        wwn,
        ata_major: w(80),
        ata_minor: w(81),
        sata_caps,
        sata_current: (sata_caps.is_some() && valid(w(77)) && cur != 0).then_some(cur),
        sata_version: (valid(w222) && w222 >> 12 == 1).then_some(w222),
        trim,
        trim_deterministic: trim == Some(true) && w(69) & (1 << 14) != 0,
        trim_zeroed: trim == Some(true) && w(69) & (1 << 5) != 0,
        smart_supported: valid(w(82)) && w(82) & 1 != 0,
        smart_enabled: valid(w(85)) && w(85) & 1 != 0,
        self_test_supported: (w84_ok && w(84) & 2 != 0) || (w87_ok && w(87) & 2 != 0),
        error_log_supported: (w84_ok && w(84) & 1 != 0) || (w87_ok && w(87) & 1 != 0),
        gpl_supported: (w84_ok && w(84) & 0x20 != 0) || (w87_ok && w(87) & 0x20 != 0),
        sct_supported: valid(w(206)) && w(206) & 1 != 0,
        checksum_ok: (w(255) & 0xff == 0xa5).then(|| checksum_ok(b)),
    }
}

pub fn form_factor_name(v: u8) -> Option<&'static str> {
    Some(match v {
        1 => "5.25 inches",
        2 => "3.5 inches",
        3 => "2.5 inches",
        4 => "1.8 inches",
        5 => "< 1.8 inches",
        6 => "mSATA",
        7 => "M.2",
        8 => "MicroSSD",
        9 => "CFast",
        _ => return None,
    })
}

/// Highest ATA standard advertised in word 80.
pub fn ata_major_name(w80: u16) -> Option<&'static str> {
    if !valid(w80) {
        return None;
    }
    const NAMES: [&str; 13] = [
        "",
        "ATA-1",
        "ATA-2",
        "ATA-3",
        "ATA/ATAPI-4",
        "ATA/ATAPI-5",
        "ATA/ATAPI-6",
        "ATA/ATAPI-7",
        "ATA8-ACS",
        "ACS-2",
        "ACS-3",
        "ACS-4",
        "ACS-5",
    ];
    (1..NAMES.len())
        .rev()
        .find(|&bit| w80 & (1 << bit) != 0)
        .map(|bit| NAMES[bit])
}

/// smartctl-style ATA version string from words 80/81.
pub fn ata_version_string(w80: u16, w81: u16) -> Option<String> {
    let minor = match w81 {
        0x0028 => Some("ATA8-ACS T13/1699-D revision 6"),
        0x0110 => Some("ACS-2 T13/2015-D revision 3"),
        0x011b => Some("ACS-3 T13/2161-D revision 4"),
        0x006d => Some("ACS-3 T13/2161-D revision 5"),
        0x005e => Some("ACS-4 T13/BSR INCITS 529 revision 5"),
        _ => None,
    };
    let major = ata_major_name(w80)?;
    Some(match minor {
        Some(m) => m.to_owned(),
        None if valid(w81) => format!("{major} (minor revision 0x{w81:04x})"),
        None => format!("{major} (minor revision not indicated)"),
    })
}

/// SATA version name from word 222.
pub fn sata_version_name(w222: u16) -> Option<&'static str> {
    const NAMES: [&str; 11] = [
        "ATA8-AST",
        "SATA 1.0a",
        "SATA II Ext",
        "SATA 2.5",
        "SATA 2.6",
        "SATA 3.0",
        "SATA 3.1",
        "SATA 3.2",
        "SATA 3.3",
        "SATA 3.4",
        "SATA 3.5",
    ];
    (0..NAMES.len())
        .rev()
        .find(|&bit| w222 & (1 << bit) != 0)
        .map(|bit| NAMES[bit])
}

/// SATA generation (1..=3) -> (display string, units of 100 Mb/s).
pub fn sata_speed(generation: u8) -> Option<(&'static str, u32)> {
    match generation {
        1 => Some(("1.5 Gb/s", 15)),
        2 => Some(("3.0 Gb/s", 30)),
        3 => Some(("6.0 Gb/s", 60)),
        _ => None,
    }
}

/// Highest SATA generation in word 76 (bits 1..=3).
pub fn sata_max_generation(w76: u16) -> Option<u8> {
    (1..=3u8).rev().find(|&g| w76 & (1 << g) != 0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SmartAttr {
    pub id: u8,
    pub flags: u16,
    pub value: u8,
    pub worst: u8,
    pub raw: [u8; 6],
}

#[derive(Debug, Clone, Default)]
pub struct SmartData {
    pub revision: u16,
    pub attrs: Vec<SmartAttr>,
    pub offline_status: u8,
    pub self_test_status: u8,
    pub offline_seconds: u16,
    pub offline_caps: u8,
    pub smart_caps: u16,
    pub errlog_caps: u8,
    pub short_minutes: u8,
    pub extended_minutes: u16,
    pub conveyance_minutes: u8,
    pub checksum_ok: bool,
}

impl SmartData {
    pub fn self_tests_supported(&self) -> bool {
        self.offline_caps & 0x10 != 0
    }
    pub fn conveyance_supported(&self) -> bool {
        self.offline_caps & 0x20 != 0
    }
    pub fn selective_supported(&self) -> bool {
        self.offline_caps & 0x40 != 0
    }
    pub fn error_logging_supported(&self) -> bool {
        self.errlog_caps & 1 != 0
    }
}

pub fn parse_smart_data(b: &[u8]) -> SmartData {
    let attrs = (0..30)
        .map(|i| &b[2 + 12 * i..14 + 12 * i])
        .filter(|e| e[0] != 0)
        .map(|e| SmartAttr {
            id: e[0],
            flags: u16::from_le_bytes([e[1], e[2]]),
            value: e[3],
            worst: e[4],
            raw: e[5..11].try_into().expect("6 bytes"),
        })
        .collect();
    let ext = if b[373] == 0xff {
        u16::from_le_bytes([b[375], b[376]])
    } else {
        b[373] as u16
    };
    SmartData {
        revision: word(b, 0),
        attrs,
        offline_status: b[362],
        self_test_status: b[363],
        offline_seconds: u16::from_le_bytes([b[364], b[365]]),
        offline_caps: b[367],
        smart_caps: u16::from_le_bytes([b[368], b[369]]),
        errlog_caps: b[370],
        short_minutes: b[372],
        extended_minutes: ext,
        conveyance_minutes: b[374],
        checksum_ok: checksum_ok(b),
    }
}

/// (attribute ID, threshold) pairs.
pub fn parse_thresholds(b: &[u8]) -> Vec<(u8, u8)> {
    (0..30)
        .map(|i| (b[2 + 12 * i], b[3 + 12 * i]))
        .filter(|&(id, _)| id != 0)
        .collect()
}

/// smartctl "WHEN_FAILED": "now", "past" or "".
pub fn when_failed(value: u8, worst: u8, thresh: u8) -> &'static str {
    if thresh == 0 {
        ""
    } else if value <= thresh {
        "now"
    } else if worst <= thresh {
        "past"
    } else {
        ""
    }
}

/// smartctl attribute flag string, e.g. "PO--CK ".
pub fn attr_flags_string(f: u16) -> String {
    let c = |bit: u16, ch: char| if f & (1 << bit) != 0 { ch } else { '-' };
    format!(
        "{}{}{}{}{}{} ",
        c(0, 'P'),
        c(1, 'O'),
        c(2, 'S'),
        c(3, 'R'),
        c(4, 'C'),
        c(5, 'K')
    )
}

/// smartctl text for SMART data byte 363 (self-test execution status).
pub fn self_test_exec_string(v: u8) -> String {
    match v >> 4 {
        0 => "completed without error".into(),
        1 => "was aborted by the host".into(),
        2 => "was interrupted by the host with a hard or soft reset".into(),
        3 => "could not complete due to a fatal or unknown error".into(),
        4 => "completed having a test element that failed and the test element that failed is not known".into(),
        5 => "completed having the electrical element of the test failed".into(),
        6 => "completed having the servo (and/or seek) test element of the test failed".into(),
        7 => "completed having the read element of the test failed".into(),
        8 => "completed having a test element that failed and the device is suspected of having handling damage".into(),
        15 => format!("in progress, {}0% remaining", v & 0x0f),
        s => format!("unknown status (0x{s:x})"),
    }
}

/// Pass/fail for a self-test status nibble; None for aborted, interrupted,
/// in progress or unknown.
pub fn self_test_passed(status_nibble: u8) -> Option<bool> {
    match status_nibble {
        0 => Some(true),
        3..=8 => Some(false),
        _ => None,
    }
}

pub fn offline_status_string(v: u8) -> &'static str {
    match v & 0x7f {
        0x00 => "was never started",
        0x02 => "was completed without error",
        0x03 => "is in progress",
        0x04 => "was suspended by an interrupting command from host",
        0x05 => "was aborted by an interrupting command from host",
        0x06 => "was aborted by the device with a fatal error",
        _ => "is in a vendor specific state",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelfTestEntry {
    pub kind: u8,
    pub status: u8,
    pub hours: u16,
    pub lba: u64,
}

#[derive(Debug, Clone, Default)]
pub struct SelfTestLog {
    pub revision: u16,
    pub sectors: usize,
    /// Newest first.
    pub entries: Vec<SelfTestEntry>,
}

/// Walks a circular descriptor list backwards from the 1-based `index`.
fn newest_first<T>(descs: Vec<Option<T>>, index: usize) -> Vec<T> {
    let n = descs.len();
    if index == 0 || index > n {
        return Vec::new();
    }
    let mut descs = descs;
    (0..n)
        .filter_map(|k| descs[(index - 1 + n - k) % n].take())
        .collect()
}

/// SMART self-test log (log 0x06, 21 x 24-byte descriptors).
pub fn parse_selftest_log(b: &[u8]) -> SelfTestLog {
    let descs = (0..21)
        .map(|i| &b[2 + 24 * i..26 + 24 * i])
        .map(|d| {
            d.iter().any(|&x| x != 0).then(|| SelfTestEntry {
                kind: d[0],
                status: d[1],
                hours: u16::from_le_bytes([d[2], d[3]]),
                lba: u32::from_le_bytes(d[5..9].try_into().expect("4 bytes")) as u64,
            })
        })
        .collect();
    SelfTestLog {
        revision: word(b, 0),
        sectors: 1,
        entries: newest_first(descs, b[508] as usize),
    }
}

/// Descriptors per extended self-test log page.
pub const EXT_SELFTEST_PER_PAGE: usize = 19;

/// Pages of a `total_pages` extended self-test log holding the newest
/// descriptors, newest first, wrapping backwards from the page holding the
/// 1-based descriptor `index`. Empty if the index is outside the log.
pub fn ext_selftest_pages_to_read(index: u16, total_pages: u16, max: u16) -> Vec<u16> {
    let (index, total) = (index as usize, total_pages as usize);
    if index == 0 || index > total * EXT_SELFTEST_PER_PAGE {
        return Vec::new();
    }
    let newest = (index - 1) / EXT_SELFTEST_PER_PAGE;
    (0..total.min(max as usize))
        .map(|k| ((newest + total - k) % total) as u16)
        .collect()
}

/// Extended self-test log (GP log 0x07, 19 x 26-byte descriptors per page;
/// revision and descriptor index in page 0). Walks newest first across the
/// whole circular log and stops at the first page missing from `pages`.
pub fn parse_ext_selftest_pages(
    page0: &[u8],
    total_pages: u16,
    pages: &std::collections::BTreeMap<u16, Vec<u8>>,
) -> SelfTestLog {
    let index = word(page0, 1) as usize;
    let total = total_pages as usize * EXT_SELFTEST_PER_PAGE;
    let mut entries = Vec::new();
    if (1..=total).contains(&index) {
        for k in 0..total {
            let pos = (index - 1 + total - k) % total;
            let Some(pg) = pages.get(&((pos / EXT_SELFTEST_PER_PAGE) as u16)) else {
                break;
            };
            let o = 4 + 26 * (pos % EXT_SELFTEST_PER_PAGE);
            let d = &pg[o..o + 26];
            if d.iter().any(|&x| x != 0) {
                entries.push(SelfTestEntry {
                    kind: d[0],
                    status: d[1],
                    hours: u16::from_le_bytes([d[2], d[3]]),
                    lba: (0..6).fold(0u64, |a, i| a | (d[5 + i] as u64) << (8 * i)),
                });
            }
        }
    }
    SelfTestLog {
        revision: page0[0] as u16,
        sectors: total_pages as usize,
        entries,
    }
}

pub fn self_test_type_string(t: u8) -> String {
    match t {
        0x00 => "Offline".into(),
        0x01 => "Short offline".into(),
        0x02 => "Extended offline".into(),
        0x03 => "Conveyance offline".into(),
        0x04 => "Selective offline".into(),
        0x7f => "Abort offline test".into(),
        0x81 => "Short captive".into(),
        0x82 => "Extended captive".into(),
        0x83 => "Conveyance captive".into(),
        0x84 => "Selective captive".into(),
        0x40..=0x7e => format!("Vendor offline (0x{t:02x})"),
        0x90..=0xff => format!("Vendor captive (0x{t:02x})"),
        _ => format!("Reserved (0x{t:02x})"),
    }
}

/// smartctl self-test log status text for a status nibble.
pub fn self_test_log_status_string(nibble: u8) -> String {
    match nibble {
        0 => "Completed without error".into(),
        1 => "Aborted by host".into(),
        2 => "Interrupted (host reset)".into(),
        3 => "Fatal or unknown error".into(),
        4 => "Completed: unknown failure".into(),
        5 => "Completed: electrical failure".into(),
        6 => "Completed: servo/seek failure".into(),
        7 => "Completed: read failure".into(),
        8 => "Completed: handling damage??".into(),
        15 => "Self-test routine in progress".into(),
        n => format!("Unknown status (0x{n:x})"),
    }
}

/// Page counts per log address from a log directory (GP 0x00 or SMART 0x00).
pub fn parse_log_directory(b: &[u8]) -> [u16; 256] {
    let mut d = [0u16; 256];
    for (i, n) in d.iter_mut().enumerate().skip(1) {
        *n = word(b, i);
    }
    d
}

/// Device error count from the summary SMART error log (log 0x01).
pub fn parse_error_log_count(b: &[u8]) -> (u8, u16) {
    (b[0], u16::from_le_bytes([b[452], b[453]]))
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SctStatus {
    pub format_version: u16,
    pub sct_version: u16,
    pub device_state: u8,
    pub current: Option<i8>,
    pub power_cycle_min: Option<i8>,
    pub power_cycle_max: Option<i8>,
    pub lifetime_min: Option<i8>,
    pub lifetime_max: Option<i8>,
}

/// SCT status (log 0xE0). None if the format version is unknown.
pub fn parse_sct_status(b: &[u8]) -> Option<SctStatus> {
    let fv = word(b, 0);
    if !(2..=3).contains(&fv) {
        return None;
    }
    let t = |o: usize| Some(b[o] as i8).filter(|&v| v != i8::MIN);
    Some(SctStatus {
        format_version: fv,
        sct_version: word(b, 1),
        device_state: b[10],
        current: t(200),
        power_cycle_min: t(201),
        power_cycle_max: t(202),
        lifetime_min: t(203),
        lifetime_max: t(204),
    })
}

/// Device statistics entry size: bytes, negative = signed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DevStatInfo {
    pub name: &'static str,
    pub size: i8,
}

pub fn devstat_page_name(page: u8) -> &'static str {
    match page {
        0 => "List of supported log pages",
        1 => "General Statistics",
        2 => "Free-Fall Statistics",
        3 => "Rotating Media Statistics",
        4 => "General Errors Statistics",
        5 => "Temperature Statistics",
        6 => "Transport Statistics",
        7 => "Solid State Device Statistics",
        _ => "Unknown Statistics",
    }
}

/// Known statistics for a page, indexed by (offset / 8 - 1).
pub fn devstat_infos(page: u8) -> &'static [DevStatInfo] {
    macro_rules! s {
        ($($size:expr, $name:expr;)*) => { &[$(DevStatInfo { name: $name, size: $size }),*] };
    }
    match page {
        1 => s![
            4, "Lifetime Power-On Resets";
            4, "Power-on Hours";
            6, "Logical Sectors Written";
            6, "Number of Write Commands";
            6, "Logical Sectors Read";
            6, "Number of Read Commands";
            6, "Date and Time TimeStamp";
            4, "Pending Error Count";
            2, "Workload Utilization";
            6, "Utilization Usage Rate";
            7, "Resource Availability";
            1, "Random Write Resources Used";
        ],
        2 => s![
            4, "Number of Free-Fall Events Detected";
            4, "Overlimit Shock Events";
        ],
        3 => s![
            4, "Spindle Motor Power-on Hours";
            4, "Head Flying Hours";
            4, "Head Load Events";
            4, "Number of Reallocated Logical Sectors";
            4, "Read Recovery Attempts";
            4, "Number of Mechanical Start Failures";
            4, "Number of Realloc. Candidate Logical Sectors";
            4, "Number of High Priority Unload Events";
        ],
        4 => s![
            4, "Number of Reported Uncorrectable Errors";
            4, "Resets Between Cmd Acceptance and Completion";
            4, "Physical Element Status Changed";
        ],
        5 => s![
            -1, "Current Temperature";
            -1, "Average Short Term Temperature";
            -1, "Average Long Term Temperature";
            -1, "Highest Temperature";
            -1, "Lowest Temperature";
            -1, "Highest Average Short Term Temperature";
            -1, "Lowest Average Short Term Temperature";
            -1, "Highest Average Long Term Temperature";
            -1, "Lowest Average Long Term Temperature";
            4, "Time in Over-Temperature";
            -1, "Specified Maximum Operating Temperature";
            4, "Time in Under-Temperature";
            -1, "Specified Minimum Operating Temperature";
        ],
        6 => s![
            4, "Number of Hardware Resets";
            4, "Number of ASR Events";
            4, "Number of Interface CRC Errors";
        ],
        7 => s![
            1, "Percentage Used Endurance Indicator";
        ],
        _ => &[],
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevStat {
    pub offset: u16,
    pub info: DevStatInfo,
    pub flags: u8,
    /// None unless the VALID flag is set.
    pub value: Option<i64>,
}

impl DevStat {
    pub fn valid(&self) -> bool {
        self.flags & 0x40 != 0
    }
    pub fn normalized(&self) -> bool {
        self.flags & 0x20 != 0
    }
    pub fn supports_dsn(&self) -> bool {
        self.flags & 0x10 != 0
    }
    pub fn condition_met(&self) -> bool {
        self.flags & 0x08 != 0
    }
    /// smartctl flag string, e.g. "V---".
    pub fn flags_string(&self) -> String {
        let c = |b: bool, ch: char| if b { ch } else { '-' };
        format!(
            "{}{}{}{}",
            c(self.valid(), 'V'),
            c(self.normalized(), 'N'),
            c(self.supports_dsn(), 'D'),
            c(self.condition_met(), 'C')
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevStatPage {
    pub number: u8,
    pub revision: u16,
    pub stats: Vec<DevStat>,
}

/// Supported page numbers from device statistics page 0.
pub fn parse_devstat_page_list(b: &[u8]) -> Vec<u8> {
    let n = b[8] as usize;
    b[9..(9 + n).min(512)]
        .iter()
        .copied()
        .filter(|&p| p != 0)
        .collect()
}

/// One device statistics page; None if the header doesn't match `page`.
pub fn parse_devstat_page(b: &[u8], page: u8) -> Option<DevStatPage> {
    let hdr = qword(b, 0);
    if (hdr >> 16) as u8 != page {
        return None;
    }
    let stats = devstat_infos(page)
        .iter()
        .enumerate()
        .filter_map(|(i, &info)| {
            let off = 8 * (i + 1);
            let q = qword(b, off);
            if q & (1 << 63) == 0 {
                return None;
            }
            let flags = (q >> 56) as u8;
            let value = (flags & 0x40 != 0).then(|| match info.size {
                -1 => q as u8 as i8 as i64,
                n => (q & ((1u64 << (8 * n as u32)) - 1)) as i64,
            });
            Some(DevStat {
                offset: off as u16,
                info,
                flags,
                value,
            })
        })
        .collect();
    Some(DevStatPage {
        number: page,
        revision: hdr as u16,
        stats,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::testutil::*;

    #[test]
    fn identify_fields() {
        let id = parse_identify(&identify_sector(&IdentifySpec::default()));
        assert_eq!(id.model, "WDC WD80EFAX-68KNBN0");
        assert_eq!(id.serial, "VAGABCDE");
        assert_eq!(id.firmware, "81.00A81");
        assert_eq!(id.sectors, 15_628_053_168);
        assert_eq!((id.logical_sector, id.physical_sector), (512, 4096));
        assert_eq!(id.rotation, Some(5400));
        assert_eq!(id.form_factor, Some(2));
        assert_eq!(
            id.wwn,
            Some(Wwn {
                naa: 5,
                oui: 0x0014ee,
                id: 0x2b1234567
            })
        );
        assert_eq!(
            ata_version_string(id.ata_major, id.ata_minor).as_deref(),
            Some("ACS-3 T13/2161-D revision 5")
        );
        assert_eq!(
            id.sata_version.and_then(sata_version_name),
            Some("SATA 3.1")
        );
        assert_eq!(id.sata_caps.and_then(sata_max_generation), Some(3));
        assert_eq!(id.sata_current, Some(3));
        assert_eq!(id.trim, Some(false));
        assert!(
            id.smart_supported && id.smart_enabled && id.gpl_supported && id.self_test_supported
        );
        assert!(id.sct_supported && id.error_log_supported);
        assert_eq!(id.checksum_ok, Some(true));
    }

    #[test]
    fn identify_ssd_lba28_and_bad_checksum() {
        let spec = IdentifySpec {
            sectors: 156_301_488,
            lba48: false,
            rotation: 1,
            form_factor: 3,
            trim: true,
            physical_exp: 0,
            ..Default::default()
        };
        let mut b = identify_sector(&spec);
        let id = parse_identify(&b);
        assert_eq!(id.sectors, 156_301_488);
        assert_eq!(id.rotation, Some(1));
        assert_eq!(id.physical_sector, 512);
        assert_eq!(id.trim, Some(true));
        assert!(id.trim_deterministic);
        assert_eq!(form_factor_name(3), Some("2.5 inches"));
        b[100] ^= 1;
        assert_eq!(parse_identify(&b).checksum_ok, Some(false));
    }

    #[test]
    fn ata_string_swaps_bytes() {
        let mut b = vec![0u8; 512];
        b[54] = b'D';
        b[55] = b'W';
        b[56] = b'C';
        b[57] = b' ';
        assert_eq!(ata_string(&b, 27, 28), "WD C");
    }

    #[test]
    fn version_names() {
        assert_eq!(ata_major_name(0x01f0), Some("ATA8-ACS"));
        assert_eq!(ata_major_name(0xffff), None);
        assert_eq!(
            ata_version_string(0x0ff0, 0).as_deref(),
            Some("ACS-4 (minor revision not indicated)")
        );
        assert_eq!(sata_speed(2), Some(("3.0 Gb/s", 30)));
        assert_eq!(sata_max_generation(0b0110), Some(2));
    }

    #[test]
    fn smart_data_and_thresholds() {
        let attrs = sample_attrs();
        let sd = parse_smart_data(&smart_data_sector(&attrs, 0xf9));
        assert!(sd.checksum_ok);
        assert_eq!(sd.attrs.len(), attrs.len());
        assert_eq!(sd.attrs[0].id, 1);
        assert_eq!(sd.self_test_status, 0xf9);
        assert_eq!((sd.short_minutes, sd.extended_minutes), (2, 1000));
        assert!(sd.self_tests_supported() && sd.error_logging_supported());
        let th = parse_thresholds(&thresholds_sector(&attrs));
        assert_eq!(th[0], (1, 51));
        assert_eq!(th.len(), attrs.len());
    }

    #[test]
    fn when_failed_rules() {
        assert_eq!(when_failed(100, 100, 0), "");
        assert_eq!(when_failed(5, 5, 10), "now");
        assert_eq!(when_failed(100, 9, 10), "past");
        assert_eq!(when_failed(100, 100, 10), "");
        assert_eq!(attr_flags_string(0x002f), "POSR-K ");
        assert_eq!(attr_flags_string(0x000b), "PO-R-- ");
        assert_eq!(attr_flags_string(0x0032), "-O--CK ");
    }

    #[test]
    fn self_test_strings() {
        assert_eq!(self_test_exec_string(0x00), "completed without error");
        assert_eq!(self_test_exec_string(0xf3), "in progress, 30% remaining");
        assert_eq!(self_test_passed(7), Some(false));
        assert_eq!(self_test_passed(1), None);
        assert_eq!(self_test_type_string(2), "Extended offline");
        assert_eq!(self_test_log_status_string(7), "Completed: read failure");
    }

    #[test]
    fn selftest_log_order_and_wrap() {
        // Slots 21, 1, 2 used; index 2 is newest.
        let entries = [
            (21, 0x01, 0x00, 100, 0),
            (1, 0x02, 0x00, 200, 0),
            (2, 0x02, 0x74, 300, 0x1234),
        ];
        let log = parse_selftest_log(&selftest_log_sector(&entries, 2));
        let hours: Vec<u16> = log.entries.iter().map(|e| e.hours).collect();
        assert_eq!(hours, [300, 200, 100]);
        assert_eq!(log.entries[0].lba, 0x1234);
        assert_eq!(log.entries[0].status >> 4, 7);
        assert!(
            parse_selftest_log(&selftest_log_sector(&entries, 0))
                .entries
                .is_empty()
        );
    }

    fn ext_pages(b: &[u8], read: &[u16]) -> std::collections::BTreeMap<u16, Vec<u8>> {
        read.iter()
            .map(|&p| (p, b[512 * p as usize..512 * (p as usize + 1)].to_vec()))
            .collect()
    }

    #[test]
    fn ext_selftest_log() {
        let b = ext_selftest_log_pages(
            1,
            &[(1, 0x01, 0x00, 10, 0), (2, 0x02, 0x70, 20, 0x1_0000_0001)],
            2,
        );
        let log = parse_ext_selftest_pages(&b, 1, &ext_pages(&b, &[0]));
        assert_eq!(log.revision, 1);
        assert_eq!(log.entries.len(), 2);
        assert_eq!(log.entries[0].lba, 0x1_0000_0001);
        assert_eq!(log.entries[1].hours, 10);
    }

    #[test]
    fn ext_selftest_index_in_later_page() {
        // 8 pages = 152 descriptors; newest is descriptor 98 (page 5, slot 3).
        let entries: Vec<_> = (95..=98).map(|i| (i, 0x01, 0x00, i as u16, 0)).collect();
        let b = ext_selftest_log_pages(8, &entries, 98);
        let read = ext_selftest_pages_to_read(98, 8, 4);
        assert_eq!(read, [5, 4, 3, 2]);
        let log = parse_ext_selftest_pages(&b, 8, &ext_pages(&b, &read));
        let hours: Vec<u16> = log.entries.iter().map(|e| e.hours).collect();
        assert_eq!(hours, [98, 97, 96, 95]);
        assert_eq!(log.sectors, 8);
    }

    #[test]
    fn ext_selftest_wraparound() {
        let entries = [
            (2, 1, 0, 2, 0),
            (1, 1, 0, 1, 0),
            (152, 1, 0, 152, 0),
            (151, 1, 0, 151, 0),
        ];
        let b = ext_selftest_log_pages(8, &entries, 2);
        let read = ext_selftest_pages_to_read(2, 8, 4);
        assert_eq!(read, [0, 7, 6, 5]);
        let log = parse_ext_selftest_pages(&b, 8, &ext_pages(&b, &read));
        let hours: Vec<u16> = log.entries.iter().map(|e| e.hours).collect();
        assert_eq!(hours, [2, 1, 152, 151]);
        assert!(ext_selftest_pages_to_read(153, 8, 4).is_empty());
        assert!(ext_selftest_pages_to_read(0, 8, 4).is_empty());
        assert_eq!(ext_selftest_pages_to_read(5, 2, 4), [0, 1]);
    }

    #[test]
    fn devstats() {
        let p0 = devstat_page0(&[1, 5, 7]);
        assert_eq!(parse_devstat_page_list(&p0), [1, 5, 7]);
        let p1 = devstat_page(1, &[(8, 0xc0, 23), (16, 0xc0, 4242), (24, 0x80, 0)]);
        let pg = parse_devstat_page(&p1, 1).unwrap();
        assert_eq!(pg.stats.len(), 3);
        assert_eq!(pg.stats[0].info.name, "Lifetime Power-On Resets");
        assert_eq!(pg.stats[1].value, Some(4242));
        assert_eq!(pg.stats[2].value, None, "supported but not valid");
        assert_eq!(pg.stats[0].flags_string(), "V---");
        let p5 = devstat_page(5, &[(8, 0xc0, 0xfb)]);
        assert_eq!(parse_devstat_page(&p5, 5).unwrap().stats[0].value, Some(-5));
        assert!(parse_devstat_page(&p5, 4).is_none());
    }

    #[test]
    fn sct_and_dirs() {
        let s = parse_sct_status(&sct_status_sector(34, 52)).unwrap();
        assert_eq!((s.current, s.lifetime_max), (Some(34), Some(52)));
        assert_eq!(s.lifetime_min, None);
        assert!(parse_sct_status(&[0u8; 512]).is_none());
        let d = parse_log_directory(&log_directory_sector(&[(0x04, 8), (0x07, 1)]));
        assert_eq!((d[4], d[7], d[6]), (8, 1, 0));
        let mut e = vec![0u8; 512];
        e[0] = 1;
        e[452] = 3;
        assert_eq!(parse_error_log_count(&e), (1, 3));
    }
}
