//! SMART attribute names and raw-value formats (smartctl's built-in defaults,
//! plus common SSD attributes smartctl only names via its drive database).

/// How smartctl renders an attribute's 48-bit raw value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawFormat {
    /// Full 48-bit decimal.
    Raw48,
    /// Low 16 bits, other words in parentheses if nonzero.
    Raw16Raw16,
    /// Low 16 bits, "(Average N)" from word 1 if nonzero.
    Raw16Avg16,
    /// Low 24 bits, upper three bytes in parentheses if nonzero.
    Raw24Raw8,
    /// Temperature in byte 0 with optional min/max.
    TempMinMax,
}

/// Returns (name, raw format) for an attribute ID.
pub fn lookup(id: u8) -> (&'static str, RawFormat) {
    use RawFormat::*;
    match id {
        1 => ("Raw_Read_Error_Rate", Raw48),
        2 => ("Throughput_Performance", Raw48),
        3 => ("Spin_Up_Time", Raw16Avg16),
        4 => ("Start_Stop_Count", Raw48),
        5 => ("Reallocated_Sector_Ct", Raw16Raw16),
        6 => ("Read_Channel_Margin", Raw48),
        7 => ("Seek_Error_Rate", Raw48),
        8 => ("Seek_Time_Performance", Raw48),
        9 => ("Power_On_Hours", Raw24Raw8),
        10 => ("Spin_Retry_Count", Raw48),
        11 => ("Calibration_Retry_Count", Raw48),
        12 => ("Power_Cycle_Count", Raw48),
        13 => ("Read_Soft_Error_Rate", Raw48),
        170 => ("Available_Reservd_Space", Raw48),
        171 => ("Program_Fail_Count", Raw48),
        172 => ("Erase_Fail_Count", Raw48),
        173 => ("Ave_Block-Erase_Count", Raw48),
        174 => ("Unexpect_Power_Loss_Ct", Raw48),
        175 => ("Program_Fail_Count_Chip", Raw48),
        176 => ("Erase_Fail_Count_Chip", Raw48),
        177 => ("Wear_Leveling_Count", Raw48),
        178 => ("Used_Rsvd_Blk_Cnt_Chip", Raw48),
        179 => ("Used_Rsvd_Blk_Cnt_Tot", Raw48),
        180 => ("Unused_Rsvd_Blk_Cnt_Tot", Raw48),
        181 => ("Program_Fail_Cnt_Total", Raw48),
        182 => ("Erase_Fail_Count_Total", Raw48),
        183 => ("Runtime_Bad_Block", Raw48),
        184 => ("End-to-End_Error", Raw48),
        187 => ("Reported_Uncorrect", Raw48),
        188 => ("Command_Timeout", Raw48),
        189 => ("High_Fly_Writes", Raw48),
        190 => ("Airflow_Temperature_Cel", TempMinMax),
        191 => ("G-Sense_Error_Rate", Raw48),
        192 => ("Power-Off_Retract_Count", Raw48),
        193 => ("Load_Cycle_Count", Raw48),
        194 => ("Temperature_Celsius", TempMinMax),
        195 => ("Hardware_ECC_Recovered", Raw48),
        196 => ("Reallocated_Event_Count", Raw16Raw16),
        197 => ("Current_Pending_Sector", Raw48),
        198 => ("Offline_Uncorrectable", Raw48),
        199 => ("UDMA_CRC_Error_Count", Raw48),
        200 => ("Multi_Zone_Error_Rate", Raw48),
        201 => ("Soft_Read_Error_Rate", Raw48),
        202 => ("Data_Address_Mark_Errs", Raw48),
        203 => ("Run_Out_Cancel", Raw48),
        204 => ("Soft_ECC_Correction", Raw48),
        205 => ("Thermal_Asperity_Rate", Raw48),
        206 => ("Flying_Height", Raw48),
        207 => ("Spin_High_Current", Raw48),
        208 => ("Spin_Buzz", Raw48),
        209 => ("Offline_Seek_Performnce", Raw48),
        220 => ("Disk_Shift", Raw48),
        221 => ("G-Sense_Error_Rate", Raw48),
        222 => ("Loaded_Hours", Raw48),
        223 => ("Load_Retry_Count", Raw48),
        224 => ("Load_Friction", Raw48),
        225 => ("Load_Cycle_Count", Raw48),
        226 => ("Load-in_Time", Raw48),
        227 => ("Torq-amp_Count", Raw48),
        228 => ("Power-off_Retract_Count", Raw48),
        230 => ("Head_Amplitude", Raw48),
        231 => ("Temperature_Celsius", Raw48),
        232 => ("Available_Reservd_Space", Raw48),
        233 => ("Media_Wearout_Indicator", Raw48),
        235 => ("POR_Recovery_Count", Raw48),
        240 => ("Head_Flying_Hours", Raw24Raw8),
        241 => ("Total_LBAs_Written", Raw48),
        242 => ("Total_LBAs_Read", Raw48),
        250 => ("Read_Error_Retry_Rate", Raw48),
        254 => ("Free_Fall_Sensor", Raw48),
        _ => ("Unknown_Attribute", Raw48),
    }
}

/// 48-bit little-endian raw value.
pub fn raw48(raw: &[u8; 6]) -> u64 {
    raw.iter().rev().fold(0u64, |acc, &b| (acc << 8) | b as u64)
}

/// smartctl-style display string for a raw value.
pub fn format_raw(fmt: RawFormat, raw: &[u8; 6]) -> String {
    let w = |i: usize| u16::from_le_bytes([raw[2 * i], raw[2 * i + 1]]);
    match fmt {
        RawFormat::Raw48 => raw48(raw).to_string(),
        RawFormat::Raw16Raw16 => {
            if w(1) != 0 || w(2) != 0 {
                format!("{} ({} {})", w(0), w(2), w(1))
            } else {
                w(0).to_string()
            }
        }
        RawFormat::Raw16Avg16 => {
            if w(1) != 0 {
                format!("{} (Average {})", w(0), w(1))
            } else {
                w(0).to_string()
            }
        }
        RawFormat::Raw24Raw8 => {
            let low = raw48(raw) & 0xFF_FFFF;
            if raw[3] != 0 || raw[4] != 0 || raw[5] != 0 {
                format!("{low} ({} {} {})", raw[5], raw[4], raw[3])
            } else {
                low.to_string()
            }
        }
        RawFormat::TempMinMax => {
            let t = raw[0] as i8;
            let (lo, hi) = (raw[2] as i8, raw[4] as i8);
            if raw[1..].iter().all(|&b| b == 0) {
                t.to_string()
            } else if raw[1] == 0 && raw[3] == 0 && raw[5] == 0 && lo <= t && t <= hi {
                format!("{t} (Min/Max {lo}/{hi})")
            } else {
                format!(
                    "{t} ({} {} {} {} {})",
                    raw[5], raw[4], raw[3], raw[2], raw[1]
                )
            }
        }
    }
}

/// Value used for `power_on_time.hours` / `temperature.current` style summaries.
pub fn primary_value(fmt: RawFormat, raw: &[u8; 6]) -> u64 {
    match fmt {
        RawFormat::Raw48 => raw48(raw),
        RawFormat::Raw16Raw16 | RawFormat::Raw16Avg16 => {
            u16::from_le_bytes([raw[0], raw[1]]) as u64
        }
        RawFormat::Raw24Raw8 => raw48(raw) & 0xFF_FFFF,
        RawFormat::TempMinMax => raw[0] as u64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(lookup(5).0, "Reallocated_Sector_Ct");
        assert_eq!(lookup(199).0, "UDMA_CRC_Error_Count");
        assert_eq!(lookup(77).0, "Unknown_Attribute");
    }

    #[test]
    fn raw_formats() {
        let r = [0x39, 0x30, 0, 0, 0, 0];
        assert_eq!(format_raw(RawFormat::Raw48, &r), "12345");
        assert_eq!(
            format_raw(RawFormat::Raw24Raw8, &[1, 0, 0, 7, 0, 0]),
            "1 (0 0 7)"
        );
        assert_eq!(primary_value(RawFormat::Raw24Raw8, &[1, 0, 0, 7, 0, 0]), 1);
        assert_eq!(
            format_raw(RawFormat::TempMinMax, &[35, 0, 18, 0, 45, 0]),
            "35 (Min/Max 18/45)"
        );
        assert_eq!(
            format_raw(RawFormat::TempMinMax, &[35, 0, 0, 0, 0, 0]),
            "35"
        );
        assert_eq!(
            format_raw(RawFormat::Raw16Avg16, &[0x9a, 0x10, 0x9a, 0x10, 0, 0]),
            "4250 (Average 4250)"
        );
        assert_eq!(format_raw(RawFormat::Raw16Raw16, &[8, 0, 0, 0, 0, 0]), "8");
        assert_eq!(raw48(&[1, 0, 0, 0, 0, 1]), 0x0100_0000_0001);
    }
}
