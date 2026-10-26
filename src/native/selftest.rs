//! Self-test start and abort through the native backend: one session runs
//! IDENTIFY (so the caller can check it reached the confirmed drive) and then
//! SMART EXECUTE OFF-LINE IMMEDIATE.

use anyhow::{Context, Result, bail};

use super::ata;
use super::sat::{AtaCmd, Sat};
use super::scsi::ScsiTransport;
use crate::smartctl::TestKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestOutcome {
    pub model: String,
    pub serial: String,
    pub message: String,
}

const ABORT: u8 = 0x7f;

fn subcommand(kind: TestKind) -> u8 {
    match kind {
        TestKind::Short => 1,
        TestKind::Long => 2,
        TestKind::Conveyance => 3,
    }
}

/// Starts (`kind` set) or aborts a self-test.
pub fn run<T: ScsiTransport>(sat: &mut Sat<T>, kind: Option<TestKind>) -> Result<TestOutcome> {
    let id = ata::parse_identify(&sat.read(AtaCmd::Identify)?);
    if !id.smart_supported {
        bail!("drive does not support SMART");
    }
    if !id.smart_enabled {
        bail!("SMART is disabled on this drive");
    }
    let message = match kind {
        Some(k) => {
            let minutes = sat
                .read(AtaCmd::SmartReadData)
                .ok()
                .map(|b| ata::parse_smart_data(&b))
                .and_then(|sd| match k {
                    TestKind::Short => (sd.short_minutes > 0).then_some(sd.short_minutes as u16),
                    TestKind::Long => (sd.extended_minutes > 0).then_some(sd.extended_minutes),
                    TestKind::Conveyance => {
                        (sd.conveyance_minutes > 0).then_some(sd.conveyance_minutes as u16)
                    }
                });
            sat.non_data_plain(AtaCmd::SmartExecuteOffline(subcommand(k)))
                .context("self-test not started")?;
            match minutes {
                Some(m) => format!("{} self-test started; about {m} min", k.name()),
                None => format!("{} self-test started", k.name()),
            }
        }
        None => {
            sat.non_data_plain(AtaCmd::SmartExecuteOffline(ABORT))
                .context("abort failed")?;
            "Self-test aborted".into()
        }
    };
    Ok(TestOutcome {
        model: id.model,
        serial: id.serial,
        message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::testutil::MockDrive;

    fn offline_subcommands(sat: &Sat<MockDrive>) -> Vec<u8> {
        sat.t
            .cdbs
            .iter()
            .filter(|c| c[0] == 0x85 && c[4] == 0xd4 && c[14] == 0xb0)
            .map(|c| c[8])
            .collect()
    }

    #[test]
    fn start_identifies_then_sends_the_subcommand() {
        let mut sat = Sat::new(MockDrive::default());
        let out = run(&mut sat, Some(TestKind::Long)).unwrap();
        assert_eq!(out.model, "WDC WD80EFAX-68KNBN0");
        assert!(!out.serial.is_empty());
        assert!(
            out.message.starts_with("Extended self-test started"),
            "{}",
            out.message
        );
        assert_eq!(offline_subcommands(&sat), [2]);
        assert_eq!(sat.t.cdbs[0][14], 0xec, "IDENTIFY first");
        assert!(
            sat.t.cdbs.iter().all(|c| c[0] != 0x85 || c[2] & 0x20 == 0),
            "no CK_COND: some bridges wedge on it"
        );
    }

    #[test]
    fn abort_sends_7f() {
        let mut sat = Sat::new(MockDrive::default());
        let out = run(&mut sat, None).unwrap();
        assert_eq!(out.message, "Self-test aborted");
        assert_eq!(offline_subcommands(&sat), [0x7f]);
    }

    #[test]
    fn rejected_command_is_an_error() {
        let mut sat = Sat::new(MockDrive {
            offline_rejected: true,
            ..Default::default()
        });
        let e = run(&mut sat, Some(TestKind::Short)).unwrap_err();
        assert!(
            format!("{e:#}").starts_with("self-test not started"),
            "{e:#}"
        );
    }
}
