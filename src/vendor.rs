//! Manufacturer lookup from model strings, WWN OUIs and NVMe PCI vendor IDs.

/// Guesses the manufacturer from the model name / smartctl model family.
pub fn from_model(model: &str, family: Option<&str>) -> Option<&'static str> {
    let hay = format!("{} {}", family.unwrap_or(""), model).to_ascii_uppercase();
    let m = model.trim().to_ascii_uppercase();
    const BY_WORD: &[(&str, &str)] = &[
        ("ADATA", "ADATA"),
        ("APPLE", "Apple"),
        ("CORSAIR", "Corsair"),
        ("CRUCIAL", "Crucial (Micron)"),
        ("FUJITSU", "Fujitsu"),
        ("HGST", "HGST (Western Digital)"),
        ("HITACHI", "Hitachi"),
        ("HYNIX", "SK hynix"),
        ("INTEL", "Intel"),
        ("KINGSTON", "Kingston"),
        ("KIOXIA", "Kioxia"),
        ("LITE-ON", "Lite-On"),
        ("LITEON", "Lite-On"),
        ("MAXTOR", "Maxtor"),
        ("MICRON", "Micron"),
        ("PLEXTOR", "Plextor"),
        ("PNY", "PNY"),
        ("SABRENT", "Sabrent"),
        ("SAMSUNG", "Samsung"),
        ("SANDISK", "SanDisk"),
        ("SEAGATE", "Seagate"),
        ("SK HYNIX", "SK hynix"),
        ("SOLIDIGM", "Solidigm"),
        ("TEAM", "Team Group"),
        ("TOSHIBA", "Toshiba"),
        ("TRANSCEND", "Transcend"),
        ("WDC ", "Western Digital"),
        ("WESTERN DIGITAL", "Western Digital"),
    ];
    if let Some((_, v)) = BY_WORD.iter().find(|(k, _)| hay.contains(k)) {
        return Some(v);
    }
    const BY_PREFIX: &[(&str, &str)] = &[
        ("CT", "Crucial (Micron)"),
        ("DT01", "Toshiba"),
        ("HDS", "Hitachi"),
        ("HDT", "Hitachi"),
        ("HDWD", "Toshiba"),
        ("HTS", "Hitachi"),
        ("HUH", "HGST (Western Digital)"),
        ("HUS", "HGST (Western Digital)"),
        ("MG0", "Toshiba"),
        ("MQ0", "Toshiba"),
        ("MZ", "Samsung"),
        ("SA400", "Kingston"),
        ("SSDSC", "Intel"),
        ("ST", "Seagate"),
        ("SV300", "Kingston"),
        ("WD", "Western Digital"),
    ];
    BY_PREFIX
        .iter()
        .find(|(k, _)| m.starts_with(k))
        .map(|(_, v)| *v)
}

/// IEEE OUI from an ATA WWN (NAA 5).
pub fn from_oui(oui: u64) -> Option<&'static str> {
    Some(match oui {
        0x000039 => "Toshiba",
        0x0000f0 => "Samsung",
        0x00080d => "Toshiba",
        0x000c50 => "Seagate",
        0x000cca => "HGST (Western Digital)",
        0x0014ee => "Western Digital",
        0x001b44 => "SanDisk",
        0x002538 => "Samsung",
        0x0026b7 => "Kingston",
        0x00a075 => "Micron",
        0x50014e => "Western Digital",
        0x5cd2e4 => "Intel",
        _ => return None,
    })
}

/// NVMe PCI vendor ID.
pub fn from_pci_vendor(id: u64) -> Option<&'static str> {
    Some(match id {
        0x1cc1 => "ADATA",
        0x106b => "Apple",
        0x8086 => "Intel",
        0x2646 => "Kingston",
        0x1e0f => "Kioxia",
        0x1e4b => "Maxio",
        0x1344 => "Micron",
        0x1987 => "Phison",
        0x144d => "Samsung",
        0x15b7 => "SanDisk (Western Digital)",
        0x1bb1 => "Seagate",
        0x1d97 => "Shenzhen Longsys (Lexar)",
        0x126f => "Silicon Motion",
        0x1c5c => "SK hynix",
        0x025e => "Solidigm",
        0x1179 => "Toshiba",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models() {
        assert_eq!(
            from_model("WDC WD40EFRX-68N32N0", None),
            Some("Western Digital")
        );
        assert_eq!(from_model("ST3000DM001-1CH166", None), Some("Seagate"));
        assert_eq!(from_model("Samsung SSD 870 EVO 1TB", None), Some("Samsung"));
        assert_eq!(
            from_model("CT1000MX500SSD1", None),
            Some("Crucial (Micron)")
        );
        assert_eq!(from_model("XYZ123", Some("Toshiba P300")), Some("Toshiba"));
        assert_eq!(from_model("XYZ123", None), None);
    }
}
