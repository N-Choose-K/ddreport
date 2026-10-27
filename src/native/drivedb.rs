//! Model family and vendor attribute names from the smartmontools drive
//! database, via `smartctl -P showall MODEL FIRMWARE` (touches no device).

use std::process::Stdio;

/// One drivedb.h entry matched against a model/firmware pair.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DbEntry {
    pub family: Option<String>,
    /// Vendor-specific attribute names, (id, name).
    pub names: Vec<(u8, String)>,
}

impl DbEntry {
    pub fn name(&self, id: u8) -> Option<&str> {
        self.names
            .iter()
            .find(|(i, _)| *i == id)
            .map(|(_, n)| n.as_str())
    }
}

/// Runs smartctl and parses its answer. `Err` means smartctl could not run;
/// `Ok(None)` means the drive has no entry.
pub fn lookup(model: &str, firmware: &str) -> std::io::Result<Option<DbEntry>> {
    let out = crate::smartctl::smartctl()
        .args(["-P", "showall", model, firmware])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()?;
    Ok(parse(&String::from_utf8_lossy(&out.stdout)))
}

/// Parses `smartctl -P showall` output.
pub fn parse(text: &str) -> Option<DbEntry> {
    if !text.contains("Drive found in smartmontools Database") {
        return None;
    }
    let mut e = DbEntry::default();
    let mut in_options = false;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("MODEL FAMILY:") {
            e.family = Some(rest.trim().to_string()).filter(|s| !s.is_empty());
            in_options = false;
        } else if let Some(rest) = line.strip_prefix("ATTRIBUTE OPTIONS:") {
            in_options = true;
            push_name(&mut e, rest);
        } else if in_options && line.starts_with(' ') {
            push_name(&mut e, line);
        } else {
            in_options = false;
        }
    }
    Some(e)
}

/// "  246 Total_LBAs_Written" -> (246, "Total_LBAs_Written").
fn push_name(e: &mut DbEntry, line: &str) {
    let mut parts = line.split_whitespace();
    let (Some(id), Some(name)) = (parts.next(), parts.next()) else {
        return;
    };
    if let Ok(id) = id.parse::<u8>() {
        e.names.push((id, name.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CRUCIAL: &str = "\
Drive found in smartmontools Database.  Drive identity strings:
MODEL:              Crucial_CT1050MX300SSD1
FIRMWARE:           M0CR021
match smartmontools Drive Database entry:
MODEL REGEXP:       Crucial_CT(275|525|750|1050|2050)MX300SSD[14]
FIRMWARE REGEXP:    .*
MODEL FAMILY:       Crucial/Micron Client SSDs
ATTRIBUTE OPTIONS:  005 Reallocate_NAND_Blk_Cnt
                    246 Total_LBAs_Written
                    247 Host_Program_Page_Count
";

    #[test]
    fn parses_family_and_names() {
        let e = parse(CRUCIAL).unwrap();
        assert_eq!(e.family.as_deref(), Some("Crucial/Micron Client SSDs"));
        assert_eq!(e.names.len(), 3);
        assert_eq!(e.name(5), Some("Reallocate_NAND_Blk_Cnt"));
        assert_eq!(e.name(246), Some("Total_LBAs_Written"));
        assert_eq!(e.name(9), None);
    }

    #[test]
    fn entry_without_options() {
        let e = parse(
            "Drive found in smartmontools Database.  Drive identity strings:\n\
             MODEL FAMILY:       Western Digital Red Plus\n\
             ATTRIBUTE OPTIONS:  None preset; no -v options are required.\n",
        )
        .unwrap();
        assert_eq!(e.family.as_deref(), Some("Western Digital Red Plus"));
        assert!(e.names.is_empty());
    }

    #[test]
    fn unknown_drive() {
        assert_eq!(
            parse("No presets are defined for this drive.  Its identity strings:\nMODEL: X\n"),
            None
        );
    }
}
