//! Parsers for ATA IDENTIFY DEVICE, SMART data/thresholds, self-test logs,
//! device statistics, SCT status and log directories (ACS-3/ACS-4 layouts).
//!
//! Offsets and bit positions come from ACS-4 (T13/BSR INCITS 529) unless a
//! constant says otherwise. Each layout gets its own `const` block below.

const SECTOR: usize = 512;

/// IDENTIFY DEVICE word numbers (ACS-4 IDENTIFY DEVICE data). Ranges are inclusive.
mod id {
    pub const SERIAL: (usize, usize) = (10, 19);
    pub const FIRMWARE: (usize, usize) = (23, 26);
    pub const MODEL: (usize, usize) = (27, 46);
    /// Words 60..=61: total addressable sectors for 28-bit commands.
    pub const LBA28_SECTORS: usize = 60;
    /// Word 69: additional supported features.
    pub const ADDITIONAL_SUPPORTED: usize = 69;
    pub const EXT_SECTORS_REPORTED: u16 = 1 << 3;
    pub const TRIM_ZEROED: u16 = 1 << 5;
    pub const TRIM_DETERMINISTIC: u16 = 1 << 14;
    /// Word 76: SATA capabilities. Bits 1..=3 = gen 1..=3 signalling speed.
    pub const SATA_CAPABILITIES: usize = 76;
    pub const SATA_GEN_FIRST_BIT: u8 = 1;
    pub const SATA_GEN_LAST_BIT: u8 = 3;
    /// Word 77: SATA additional capabilities. Bits 3:1 = current speed.
    pub const SATA_ADDITIONAL: usize = 77;
    pub const SATA_CURRENT_SHIFT: u16 = 1;
    pub const SATA_CURRENT_MASK: u16 = 0x7;
    pub const MAJOR_VERSION: usize = 80;
    pub const MINOR_VERSION: usize = 81;
    /// Words 82..=84: command sets supported; 85..=87: enabled.
    pub const SUPPORTED_1: usize = 82;
    pub const SUPPORTED_2: usize = 83;
    pub const SUPPORTED_3: usize = 84;
    pub const ENABLED_1: usize = 85;
    pub const ENABLED_3: usize = 87;
    /// Word 82 / 85 bit 0.
    pub const SMART: u16 = 1 << 0;
    /// Word 83 bit 10.
    pub const LBA48: u16 = 1 << 10;
    /// Word 84 / 87 bits.
    pub const SMART_ERROR_LOG: u16 = 1 << 0;
    pub const SMART_SELF_TEST: u16 = 1 << 1;
    pub const GPL: u16 = 1 << 5;
    pub const WWN_64BIT: u16 = 1 << 8;
    /// Words 100..=103: total addressable sectors for 48-bit commands.
    pub const LBA48_SECTORS: usize = 100;
    /// Word 106: physical/logical sector size.
    pub const SECTOR_SIZE: usize = 106;
    pub const LOGICAL_SECTOR_LONGER_THAN_256_WORDS: u16 = 1 << 12;
    pub const MULTIPLE_LOGICAL_PER_PHYSICAL: u16 = 1 << 13;
    pub const LOGICAL_PER_PHYSICAL_EXP_MASK: u16 = 0x0f;
    /// Words 108..=111: 64-bit world wide name, most significant word first.
    /// Bits 63:60 = NAA, 59:36 = OUI, 35:0 = unique ID.
    pub const WWN: usize = 108;
    pub const WWN_NAA_SHIFT: u64 = 60;
    pub const WWN_OUI_SHIFT: u64 = 36;
    pub const WWN_OUI_MASK: u64 = (1 << 24) - 1;
    pub const WWN_ID_MASK: u64 = (1 << 36) - 1;
    /// Words 117..=118: logical sector size in words.
    pub const LOGICAL_SECTOR_WORDS: usize = 117;
    pub const MIN_LOGICAL_SECTOR_WORDS: u32 = 256;
    /// Word 168 bits 3:0: nominal form factor.
    pub const FORM_FACTOR: usize = 168;
    pub const FORM_FACTOR_MASK: u16 = 0x0f;
    pub const FORM_FACTOR_RANGE: std::ops::RangeInclusive<u16> = 1..=9;
    /// Word 169 bit 0: DATA SET MANAGEMENT (TRIM) supported.
    pub const DATA_SET_MANAGEMENT: usize = 169;
    pub const TRIM: u16 = 1 << 0;
    /// Word 206 bit 0: SCT Command Transport supported.
    pub const SCT: usize = 206;
    pub const SCT_SUPPORTED: u16 = 1 << 0;
    /// Word 217: 1 = non-rotating, 0x401..=0xfffe = RPM.
    pub const ROTATION_RATE: usize = 217;
    pub const NON_ROTATING: u16 = 1;
    pub const RPM_MIN: u16 = 0x401;
    /// Word 222: transport major version. Bits 15:12 = transport type.
    pub const TRANSPORT_MAJOR: usize = 222;
    pub const TRANSPORT_TYPE_SHIFT: u16 = 12;
    pub const TRANSPORT_SERIAL: u16 = 1;
    /// Words 230..=233: extended number of user addressable sectors.
    pub const EXT_SECTORS: usize = 230;
    /// Word 255: low byte 0xA5 = checksum present.
    pub const INTEGRITY: usize = 255;
    pub const INTEGRITY_SIGNATURE: u16 = 0xa5;
}

/// SMART READ DATA byte offsets (ACS-4 SMART READ DATA).
mod sd {
    pub const REVISION_WORD: usize = 0;
    pub const ATTRS: usize = 2;
    pub const ATTR_COUNT: usize = 30;
    pub const ATTR_SIZE: usize = 12;
    /// Attribute entry layout.
    pub const ATTR_ID: usize = 0;
    pub const ATTR_FLAGS: usize = 1;
    pub const ATTR_VALUE: usize = 3;
    pub const ATTR_WORST: usize = 4;
    pub const ATTR_RAW: std::ops::Range<usize> = 5..11;
    /// Threshold entry: ID at 0, threshold at 1.
    pub const THRESH_VALUE: usize = 1;
    pub const OFFLINE_STATUS: usize = 362;
    pub const SELF_TEST_STATUS: usize = 363;
    pub const OFFLINE_SECONDS: usize = 364;
    pub const OFFLINE_CAPS: usize = 367;
    pub const SMART_CAPS: usize = 368;
    pub const ERRLOG_CAPS: usize = 370;
    pub const SHORT_MINUTES: usize = 372;
    pub const EXTENDED_MINUTES_BYTE: usize = 373;
    pub const EXTENDED_MINUTES_WORD: usize = 375;
    pub const EXTENDED_MINUTES_IN_WORD: u8 = 0xff;
    pub const CONVEYANCE_MINUTES: usize = 374;
    /// Offline data collection capability bits (byte 367).
    pub const CAP_SELF_TEST: u8 = 1 << 4;
    pub const CAP_CONVEYANCE: u8 = 1 << 5;
    pub const CAP_SELECTIVE: u8 = 1 << 6;
    /// Error logging capability bit (byte 370).
    pub const CAP_ERROR_LOGGING: u8 = 1 << 0;
    /// Offline status byte 362: bit 7 = auto offline enabled, bits 6:0 = status.
    pub const OFFLINE_STATUS_MASK: u8 = 0x7f;
}

/// Attribute flag bits (byte 1..=2 of an attribute entry).
mod attr_flag {
    pub const PREFAIL: u16 = 1 << 0;
    pub const ONLINE: u16 = 1 << 1;
    pub const PERFORMANCE: u16 = 1 << 2;
    pub const ERROR_RATE: u16 = 1 << 3;
    pub const EVENT_COUNT: u16 = 1 << 4;
    pub const SELF_PRESERVING: u16 = 1 << 5;
}

/// SMART self-test log (log 0x06) layout.
mod stl {
    pub const REVISION_WORD: usize = 0;
    pub const DESCRIPTORS: usize = 2;
    pub const DESC_COUNT: usize = 21;
    pub const DESC_SIZE: usize = 24;
    pub const INDEX: usize = 508;
    /// Descriptor layout, shared with the extended log.
    pub const TYPE: usize = 0;
    pub const STATUS: usize = 1;
    pub const HOURS: usize = 2;
    pub const LBA: usize = 5;
    pub const LBA_BYTES: usize = 4;
}

/// Extended self-test log (GP log 0x07) layout.
mod ext_stl {
    pub const REVISION: usize = 0;
    pub const INDEX_WORD: usize = 1;
    pub const DESCRIPTORS: usize = 4;
    pub const DESC_SIZE: usize = 26;
    pub const LBA_BYTES: usize = 6;
}

/// Summary SMART error log (log 0x01) layout.
mod errlog {
    pub const VERSION: usize = 0;
    pub const DEVICE_ERROR_COUNT: usize = 452;
}

/// SCT status (log 0xE0) layout, format versions 2 and 3.
mod sct {
    pub const FORMAT_VERSION_WORD: usize = 0;
    pub const SCT_VERSION_WORD: usize = 1;
    pub const DEVICE_STATE: usize = 10;
    pub const CURRENT_TEMP: usize = 200;
    pub const POWER_CYCLE_MIN: usize = 201;
    pub const POWER_CYCLE_MAX: usize = 202;
    pub const LIFETIME_MIN: usize = 203;
    pub const LIFETIME_MAX: usize = 204;
    pub const FORMAT_VERSIONS: std::ops::RangeInclusive<u16> = 2..=3;
    /// 0x80 as i8: temperature not reported.
    pub const TEMP_UNKNOWN: i8 = i8::MIN;
}

/// Device statistics (GP log 0x04) layout.
mod devstat {
    pub const ENTRY_SIZE: usize = 8;
    /// Page 0: byte 8 = page count, bytes 9.. = page numbers.
    pub const PAGE_LIST_COUNT: usize = 8;
    pub const PAGE_LIST: usize = 9;
    /// Page header qword: bits 15:0 revision, bits 23:16 page number.
    pub const HEADER_PAGE_SHIFT: u64 = 16;
    /// Entry qword: bits 63:56 flags; bit 63 = supported.
    pub const FLAGS_SHIFT: u64 = 56;
    pub const SUPPORTED: u64 = 1 << 63;
    pub const FLAG_VALID: u8 = 1 << 6;
    pub const FLAG_NORMALIZED: u8 = 1 << 5;
    pub const FLAG_DSN: u8 = 1 << 4;
    pub const FLAG_CONDITION_MET: u8 = 1 << 3;
}

fn word(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[2 * i], b[2 * i + 1]])
}

fn byte_word(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}

fn qword(b: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(b[off..off + 8].try_into().expect("8 bytes"))
}

/// Little-endian unsigned integer from `n` consecutive words starting at `first`.
fn words_le(b: &[u8], first: usize, n: usize) -> u64 {
    (0..n).fold(0u64, |a, i| a | (word(b, first + i) as u64) << (16 * i))
}

/// Word is "valid" in the ATA sense: not 0x0000 and not 0xFFFF.
fn valid(w: u16) -> bool {
    w != 0 && w != 0xffff
}

/// Words 83, 84, 87, 106, ...: contents are valid only when bit 15 is clear and bit 14 is set.
const VALID_PATTERN_MASK: u16 = 0xc000;
const VALID_PATTERN: u16 = 0x4000;

fn pattern_valid(w: u16) -> bool {
    w & VALID_PATTERN_MASK == VALID_PATTERN
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
    b[..SECTOR].iter().fold(0u8, |a, &x| a.wrapping_add(x)) == 0
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
    let w69 = w(id::ADDITIONAL_SUPPORTED);
    let w83 = w(id::SUPPORTED_2);
    let w84 = w(id::SUPPORTED_3);
    let w87 = w(id::ENABLED_3);
    let w106 = w(id::SECTOR_SIZE);
    // Word 84 reports support, 87 reports enabled; drives fill either.
    let w84_or_87 =
        |bit: u16| (pattern_valid(w84) && w84 & bit != 0) || (pattern_valid(w87) && w87 & bit != 0);

    let lba48 = pattern_valid(w83) && w83 & id::LBA48 != 0;
    let mut sectors = if lba48 {
        words_le(b, id::LBA48_SECTORS, 4)
    } else {
        0
    };
    if w69 & id::EXT_SECTORS_REPORTED != 0 {
        let ext = words_le(b, id::EXT_SECTORS, 4);
        if ext != 0 {
            sectors = ext;
        }
    }
    if sectors == 0 {
        sectors = words_le(b, id::LBA28_SECTORS, 2);
    }

    let w106_ok = pattern_valid(w106);
    let mut logical = 512u32;
    if w106_ok && w106 & id::LOGICAL_SECTOR_LONGER_THAN_256_WORDS != 0 {
        let words = words_le(b, id::LOGICAL_SECTOR_WORDS, 2) as u32;
        if words >= id::MIN_LOGICAL_SECTOR_WORDS {
            logical = words * 2;
        }
    }
    let physical = if w106_ok && w106 & id::MULTIPLE_LOGICAL_PER_PHYSICAL != 0 {
        logical << (w106 & id::LOGICAL_PER_PHYSICAL_EXP_MASK)
    } else {
        logical
    };

    let wwn_words = [w(id::WWN), w(id::WWN + 1), w(id::WWN + 2), w(id::WWN + 3)];
    let wwn = (pattern_valid(w87) && w87 & id::WWN_64BIT != 0 && wwn_words.iter().any(|&x| x != 0))
        .then(|| {
            let name = wwn_words.iter().fold(0u64, |a, &x| (a << 16) | x as u64);
            Wwn {
                naa: (name >> id::WWN_NAA_SHIFT) as u8,
                oui: ((name >> id::WWN_OUI_SHIFT) & id::WWN_OUI_MASK) as u32,
                id: name & id::WWN_ID_MASK,
            }
        });

    let rot = w(id::ROTATION_RATE);
    let ff = w(id::FORM_FACTOR) & id::FORM_FACTOR_MASK;
    let w76 = w(id::SATA_CAPABILITIES);
    let w77 = w(id::SATA_ADDITIONAL);
    let sata_caps = valid(w76).then_some(w76);
    let cur = ((w77 >> id::SATA_CURRENT_SHIFT) & id::SATA_CURRENT_MASK) as u8;
    let w222 = w(id::TRANSPORT_MAJOR);
    let w169 = w(id::DATA_SET_MANAGEMENT);
    let trim = (w169 != 0xffff).then_some(w169 & id::TRIM != 0);
    let (serial, firmware, model) = (id::SERIAL, id::FIRMWARE, id::MODEL);

    Identify {
        model: ata_string(b, model.0, model.1),
        serial: ata_string(b, serial.0, serial.1),
        firmware: ata_string(b, firmware.0, firmware.1),
        sectors,
        logical_sector: logical,
        physical_sector: physical,
        rotation: (rot == id::NON_ROTATING || (id::RPM_MIN..0xffff).contains(&rot)).then_some(rot),
        form_factor: id::FORM_FACTOR_RANGE.contains(&ff).then_some(ff as u8),
        wwn,
        ata_major: w(id::MAJOR_VERSION),
        ata_minor: w(id::MINOR_VERSION),
        sata_caps,
        sata_current: (sata_caps.is_some() && valid(w77) && cur != 0).then_some(cur),
        sata_version: (valid(w222) && w222 >> id::TRANSPORT_TYPE_SHIFT == id::TRANSPORT_SERIAL)
            .then_some(w222),
        trim,
        trim_deterministic: trim == Some(true) && w69 & id::TRIM_DETERMINISTIC != 0,
        trim_zeroed: trim == Some(true) && w69 & id::TRIM_ZEROED != 0,
        smart_supported: valid(w(id::SUPPORTED_1)) && w(id::SUPPORTED_1) & id::SMART != 0,
        smart_enabled: valid(w(id::ENABLED_1)) && w(id::ENABLED_1) & id::SMART != 0,
        self_test_supported: w84_or_87(id::SMART_SELF_TEST),
        error_log_supported: w84_or_87(id::SMART_ERROR_LOG),
        gpl_supported: w84_or_87(id::GPL),
        sct_supported: valid(w(id::SCT)) && w(id::SCT) & id::SCT_SUPPORTED != 0,
        checksum_ok: (w(id::INTEGRITY) & 0xff == id::INTEGRITY_SIGNATURE).then(|| checksum_ok(b)),
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

/// Highest ATA standard advertised in word 80 (bit n = standard n).
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

/// smartctl-style ATA version string from words 80/81. Minor codes are the
/// ACS-4 minor version codes smartctl prints by name.
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

/// SATA version name from word 222 (bit n = version n).
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

/// Highest SATA generation in word 76.
pub fn sata_max_generation(w76: u16) -> Option<u8> {
    (id::SATA_GEN_FIRST_BIT..=id::SATA_GEN_LAST_BIT)
        .rev()
        .find(|&g| w76 & (1 << g) != 0)
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
        self.offline_caps & sd::CAP_SELF_TEST != 0
    }
    pub fn conveyance_supported(&self) -> bool {
        self.offline_caps & sd::CAP_CONVEYANCE != 0
    }
    pub fn selective_supported(&self) -> bool {
        self.offline_caps & sd::CAP_SELECTIVE != 0
    }
    pub fn error_logging_supported(&self) -> bool {
        self.errlog_caps & sd::CAP_ERROR_LOGGING != 0
    }
}

fn attr_entries(b: &[u8]) -> impl Iterator<Item = &[u8]> {
    (0..sd::ATTR_COUNT)
        .map(move |i| &b[sd::ATTRS + sd::ATTR_SIZE * i..][..sd::ATTR_SIZE])
        .filter(|e| e[sd::ATTR_ID] != 0)
}

pub fn parse_smart_data(b: &[u8]) -> SmartData {
    let attrs = attr_entries(b)
        .map(|e| SmartAttr {
            id: e[sd::ATTR_ID],
            flags: byte_word(e, sd::ATTR_FLAGS),
            value: e[sd::ATTR_VALUE],
            worst: e[sd::ATTR_WORST],
            raw: e[sd::ATTR_RAW].try_into().expect("6 bytes"),
        })
        .collect();
    let ext = if b[sd::EXTENDED_MINUTES_BYTE] == sd::EXTENDED_MINUTES_IN_WORD {
        byte_word(b, sd::EXTENDED_MINUTES_WORD)
    } else {
        b[sd::EXTENDED_MINUTES_BYTE] as u16
    };
    SmartData {
        revision: word(b, sd::REVISION_WORD),
        attrs,
        offline_status: b[sd::OFFLINE_STATUS],
        self_test_status: b[sd::SELF_TEST_STATUS],
        offline_seconds: byte_word(b, sd::OFFLINE_SECONDS),
        offline_caps: b[sd::OFFLINE_CAPS],
        smart_caps: byte_word(b, sd::SMART_CAPS),
        errlog_caps: b[sd::ERRLOG_CAPS],
        short_minutes: b[sd::SHORT_MINUTES],
        extended_minutes: ext,
        conveyance_minutes: b[sd::CONVEYANCE_MINUTES],
        checksum_ok: checksum_ok(b),
    }
}

/// (attribute ID, threshold) pairs. Same entry grid as SMART READ DATA.
pub fn parse_thresholds(b: &[u8]) -> Vec<(u8, u8)> {
    attr_entries(b)
        .map(|e| (e[sd::ATTR_ID], e[sd::THRESH_VALUE]))
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
    let c = |bit: u16, ch: char| if f & bit != 0 { ch } else { '-' };
    format!(
        "{}{}{}{}{}{} ",
        c(attr_flag::PREFAIL, 'P'),
        c(attr_flag::ONLINE, 'O'),
        c(attr_flag::PERFORMANCE, 'S'),
        c(attr_flag::ERROR_RATE, 'R'),
        c(attr_flag::EVENT_COUNT, 'C'),
        c(attr_flag::SELF_PRESERVING, 'K')
    )
}

/// Self-test status byte: high nibble = status, low nibble = tenths remaining.
const SELF_TEST_STATUS_SHIFT: u8 = 4;
const SELF_TEST_REMAINING_MASK: u8 = 0x0f;

pub fn self_test_status_nibble(v: u8) -> u8 {
    v >> SELF_TEST_STATUS_SHIFT
}

pub const SELF_TEST_IN_PROGRESS: u8 = 15;

/// Percent of a running self-test left, from the low nibble (tenths).
pub fn self_test_remaining_percent(v: u8) -> Option<u32> {
    (self_test_status_nibble(v) == SELF_TEST_IN_PROGRESS)
        .then(|| (v & SELF_TEST_REMAINING_MASK) as u32 * 10)
}

/// smartctl text for the SMART data self-test execution status byte.
pub fn self_test_exec_string(v: u8) -> String {
    match self_test_status_nibble(v) {
        0 => "completed without error".into(),
        1 => "was aborted by the host".into(),
        2 => "was interrupted by the host with a hard or soft reset".into(),
        3 => "could not complete due to a fatal or unknown error".into(),
        4 => "completed having a test element that failed and the test element that failed is not known".into(),
        5 => "completed having the electrical element of the test failed".into(),
        6 => "completed having the servo (and/or seek) test element of the test failed".into(),
        7 => "completed having the read element of the test failed".into(),
        8 => "completed having a test element that failed and the device is suspected of having handling damage".into(),
        SELF_TEST_IN_PROGRESS => format!("in progress, {}0% remaining", v & SELF_TEST_REMAINING_MASK),
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
    match v & sd::OFFLINE_STATUS_MASK {
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

impl SelfTestEntry {
    /// From a descriptor; `lba_bytes` is 4 (SMART log) or 6 (extended log).
    fn from_descriptor(d: &[u8], lba_bytes: usize) -> Option<Self> {
        d.iter().any(|&x| x != 0).then(|| SelfTestEntry {
            kind: d[stl::TYPE],
            status: d[stl::STATUS],
            hours: byte_word(d, stl::HOURS),
            lba: (0..lba_bytes).fold(0u64, |a, i| a | (d[stl::LBA + i] as u64) << (8 * i)),
        })
    }
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

/// SMART self-test log (log 0x06).
pub fn parse_selftest_log(b: &[u8]) -> SelfTestLog {
    let descs = (0..stl::DESC_COUNT)
        .map(|i| &b[stl::DESCRIPTORS + stl::DESC_SIZE * i..][..stl::DESC_SIZE])
        .map(|d| SelfTestEntry::from_descriptor(d, stl::LBA_BYTES))
        .collect();
    SelfTestLog {
        revision: word(b, stl::REVISION_WORD),
        sectors: 1,
        entries: newest_first(descs, b[stl::INDEX] as usize),
    }
}

/// Descriptors per extended self-test log page.
pub const EXT_SELFTEST_PER_PAGE: usize = (SECTOR - ext_stl::DESCRIPTORS) / ext_stl::DESC_SIZE;

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

/// Extended self-test log (GP log 0x07; revision and descriptor index in
/// page 0). Walks newest first across the whole circular log and stops at
/// the first page missing from `pages`.
pub fn parse_ext_selftest_pages(
    page0: &[u8],
    total_pages: u16,
    pages: &std::collections::BTreeMap<u16, Vec<u8>>,
) -> SelfTestLog {
    let index = word(page0, ext_stl::INDEX_WORD) as usize;
    let total = total_pages as usize * EXT_SELFTEST_PER_PAGE;
    let mut entries = Vec::new();
    if (1..=total).contains(&index) {
        for k in 0..total {
            let pos = (index - 1 + total - k) % total;
            let Some(pg) = pages.get(&((pos / EXT_SELFTEST_PER_PAGE) as u16)) else {
                break;
            };
            let o = ext_stl::DESCRIPTORS + ext_stl::DESC_SIZE * (pos % EXT_SELFTEST_PER_PAGE);
            let d = &pg[o..o + ext_stl::DESC_SIZE];
            entries.extend(SelfTestEntry::from_descriptor(d, ext_stl::LBA_BYTES));
        }
    }
    SelfTestLog {
        revision: page0[ext_stl::REVISION] as u16,
        sectors: total_pages as usize,
        entries,
    }
}

/// Self-test type byte: bit 7 = captive mode.
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
        SELF_TEST_IN_PROGRESS => "Self-test routine in progress".into(),
        n => format!("Unknown status (0x{n:x})"),
    }
}

/// Page counts per log address from a log directory (GP 0x00 or SMART 0x00):
/// word n = page count of log n; word 0 is the directory version.
pub fn parse_log_directory(b: &[u8]) -> [u16; 256] {
    let mut d = [0u16; 256];
    for (i, n) in d.iter_mut().enumerate().skip(1) {
        *n = word(b, i);
    }
    d
}

/// (version, device error count) from the summary SMART error log (log 0x01).
pub fn parse_error_log_count(b: &[u8]) -> (u8, u16) {
    (b[errlog::VERSION], byte_word(b, errlog::DEVICE_ERROR_COUNT))
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
    let fv = word(b, sct::FORMAT_VERSION_WORD);
    if !sct::FORMAT_VERSIONS.contains(&fv) {
        return None;
    }
    let t = |o: usize| Some(b[o] as i8).filter(|&v| v != sct::TEMP_UNKNOWN);
    Some(SctStatus {
        format_version: fv,
        sct_version: word(b, sct::SCT_VERSION_WORD),
        device_state: b[sct::DEVICE_STATE],
        current: t(sct::CURRENT_TEMP),
        power_cycle_min: t(sct::POWER_CYCLE_MIN),
        power_cycle_max: t(sct::POWER_CYCLE_MAX),
        lifetime_min: t(sct::LIFETIME_MIN),
        lifetime_max: t(sct::LIFETIME_MAX),
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

/// Known statistics for a page, in entry order (entry n at byte 8 * (n + 1)).
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
        self.flags & devstat::FLAG_VALID != 0
    }
    pub fn normalized(&self) -> bool {
        self.flags & devstat::FLAG_NORMALIZED != 0
    }
    pub fn supports_dsn(&self) -> bool {
        self.flags & devstat::FLAG_DSN != 0
    }
    pub fn condition_met(&self) -> bool {
        self.flags & devstat::FLAG_CONDITION_MET != 0
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
    let n = b[devstat::PAGE_LIST_COUNT] as usize;
    b[devstat::PAGE_LIST..(devstat::PAGE_LIST + n).min(SECTOR)]
        .iter()
        .copied()
        .filter(|&p| p != 0)
        .collect()
}

/// One device statistics page; None if the header doesn't match `page`.
pub fn parse_devstat_page(b: &[u8], page: u8) -> Option<DevStatPage> {
    let hdr = qword(b, 0);
    if (hdr >> devstat::HEADER_PAGE_SHIFT) as u8 != page {
        return None;
    }
    let stats = devstat_infos(page)
        .iter()
        .enumerate()
        .filter_map(|(i, &info)| {
            let off = devstat::ENTRY_SIZE * (i + 1);
            let q = qword(b, off);
            if q & devstat::SUPPORTED == 0 {
                return None;
            }
            let flags = (q >> devstat::FLAGS_SHIFT) as u8;
            let value = (flags & devstat::FLAG_VALID != 0).then(|| match info.size {
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
