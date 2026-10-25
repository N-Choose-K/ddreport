//! Known USB-SATA bridge chipsets by VID:PID.

/// Chipset name for a VID:PID, falling back to the vendor name.
pub fn chipset(vid: u16, pid: u16) -> Option<String> {
    let exact = match (vid, pid) {
        (0x152d, 0x0539) => Some("JMicron JMS539"),
        (0x152d, 0x0561) => Some("JMicron JMS561"),
        (0x152d, 0x0562) => Some("JMicron JMS562"),
        (0x152d, 0x0567) => Some("JMicron JMS567"),
        (0x152d, 0x0578) => Some("JMicron JMS578"),
        (0x152d, 0x0583) => Some("JMicron JMS583 (NVMe)"),
        (0x152d, 0x2329) => Some("JMicron JM20329"),
        (0x152d, 0x2338) => Some("JMicron JM20338"),
        (0x152d, 0x2339) => Some("JMicron JM20339"),
        (0x174c, 0x55aa) => Some("ASMedia ASM1051E/ASM1053E/ASM1153E"),
        (0x174c, 0x1153) => Some("ASMedia ASM1153"),
        (0x174c, 0x235c) => Some("ASMedia ASM235CM"),
        (0x174c, 0x2362) => Some("ASMedia ASM2362 (NVMe)"),
        (0x0bda, 0x9210) => Some("Realtek RTL9210"),
        (0x0bda, 0x9201) => Some("Realtek RTL9201"),
        (0x1f75, 0x0611) => Some("Innostor IS611"),
        (0x1f75, 0x0621) => Some("Innostor IS621"),
        (0x2109, 0x0711) => Some("VIA Labs VL711"),
        (0x2109, 0x0715) => Some("VIA Labs VL715"),
        (0x2109, 0x0716) => Some("VIA Labs VL716"),
        (0x2537, 0x1066) => Some("Norelsys NS1066"),
        (0x2537, 0x1068) => Some("Norelsys NS1068"),
        (0x067b, 0x2773) => Some("Prolific PL2773"),
        _ => None,
    };
    exact
        .map(str::to_owned)
        .or_else(|| vendor(vid).map(|v| format!("{v} (unknown model)")))
}

/// Bridge or enclosure vendor name for a USB vendor ID.
pub fn vendor(vid: u16) -> Option<&'static str> {
    Some(match vid {
        0x0411 => "Buffalo",
        0x0480 => "Toshiba",
        0x04b4 => "Cypress",
        0x04e8 => "Samsung",
        0x04fc => "Sunplus",
        0x059f => "LaCie",
        0x05e3 => "Genesys Logic",
        0x067b => "Prolific",
        0x0bc2 => "Seagate",
        0x0bda => "Realtek",
        0x1058 => "Western Digital",
        0x13fd => "Initio",
        0x152d => "JMicron",
        0x174c => "ASMedia",
        0x1d5c => "Fresco Logic",
        0x1f75 => "Innostor",
        0x2109 => "VIA Labs",
        0x2537 => "Norelsys",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup() {
        assert_eq!(chipset(0x1f75, 0x0611).as_deref(), Some("Innostor IS611"));
        assert_eq!(chipset(0x152d, 0x0578).as_deref(), Some("JMicron JMS578"));
        assert_eq!(
            chipset(0x1058, 0x25a2).as_deref(),
            Some("Western Digital (unknown model)")
        );
        assert_eq!(chipset(0xdead, 0xbeef), None);
    }
}
