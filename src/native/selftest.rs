//! Self-test start and abort through the native backend: one session runs
//! IDENTIFY (so the caller can check it reached the confirmed drive) and then
//! SMART EXECUTE OFF-LINE IMMEDIATE.

use std::time::Duration;

use anyhow::{Context, Result, bail};

use super::ata;
use super::bot::CswTimeout;
use super::bridges::Quirks;
use super::sat::{AtaCmd, Sat};
use super::scsi::ScsiTransport;
use crate::smartctl::TestKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestOutcome {
    pub model: String,
    pub serial: String,
    pub message: String,
    /// False when the bridge gave no reply (`Quirks::silent_non_data`); the
    /// drive most likely ran the command, and a progress read settles it.
    pub confirmed: bool,
}

const ABORT: u8 = 0x7f;
/// CSW wait for a bridge that is known not to answer.
const SILENT_TIMEOUT: Duration = Duration::from_secs(3);

fn subcommand(kind: TestKind) -> u8 {
    match kind {
        TestKind::Short => 1,
        TestKind::Long => 2,
        TestKind::Conveyance => 3,
    }
}

/// The drive a command was confirmed for: model and serial from IDENTIFY.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expected {
    pub model: String,
    pub serial: String,
}

/// Starts (`kind` set) or aborts a self-test. With `expected`, IDENTIFY must
/// match before the command is sent: the selector alone can pick a swapped
/// drive.
pub fn run<T: ScsiTransport>(
    sat: &mut Sat<T>,
    kind: Option<TestKind>,
    expected: Option<&Expected>,
    quirks: Quirks,
) -> Result<TestOutcome> {
    let id = ata::parse_identify(&sat.read(AtaCmd::Identify)?);
    if let Some(e) = expected
        && (id.model != e.model || id.serial != e.serial)
    {
        bail!(
            "refused: the drive is {} (serial {}), confirmed {} (serial {})",
            id.model,
            id.serial,
            e.model,
            e.serial
        );
    }
    if !id.smart_supported {
        bail!("drive does not support SMART");
    }
    if !id.smart_enabled {
        bail!("SMART is disabled on this drive");
    }
    let timeout = if quirks.silent_non_data {
        SILENT_TIMEOUT
    } else {
        super::sat::TIMEOUT
    };
    // Ok(true): acknowledged. Ok(false): no CSW from a bridge known to stay silent.
    let send = |sat: &mut Sat<T>, sub: u8| -> Result<bool> {
        match sat.non_data_plain_within(AtaCmd::SmartExecuteOffline(sub), timeout) {
            Ok(()) => Ok(true),
            Err(e)
                if quirks.silent_non_data
                    && e.chain().any(|c| c.downcast_ref::<CswTimeout>().is_some()) =>
            {
                Ok(false)
            }
            Err(e) => Err(e),
        }
    };
    let confirmed;
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
            confirmed = send(sat, subcommand(k)).context("self-test not started")?;
            let started = match minutes {
                Some(m) => format!("{} self-test started; about {m} min", k.name()),
                None => format!("{} self-test started", k.name()),
            };
            if confirmed {
                started
            } else {
                format!("{started} (sent; this bridge never acknowledges, progress will confirm)")
            }
        }
        None => {
            confirmed = send(sat, ABORT).context("abort failed")?;
            if confirmed {
                "Self-test aborted".into()
            } else {
                "Abort sent; this bridge never acknowledges, the next read will confirm".into()
            }
        }
    };
    Ok(TestOutcome {
        model: id.model,
        serial: id.serial,
        message,
        confirmed,
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

    const Q: Quirks = Quirks {
        no_return_status: false,
        silent_non_data: false,
    };
    const SILENT: Quirks = Quirks {
        no_return_status: true,
        silent_non_data: true,
    };

    #[test]
    fn silent_bridge_counts_the_command_as_sent() {
        let mut sat = Sat::new(MockDrive {
            offline_no_csw: true,
            ..Default::default()
        });
        let e = run(&mut sat, Some(TestKind::Short), None, Q).unwrap_err();
        assert!(format!("{e:#}").contains("timed out"), "{e:#}");

        let mut sat = Sat::new(MockDrive {
            offline_no_csw: true,
            ..Default::default()
        });
        let out = run(&mut sat, Some(TestKind::Short), None, SILENT).unwrap();
        assert!(!out.confirmed);
        assert!(
            out.message.contains("never acknowledges"),
            "{}",
            out.message
        );
        let out = run(&mut sat, None, None, SILENT).unwrap();
        assert!(!out.confirmed);
        assert!(out.message.starts_with("Abort sent"));
    }

    #[test]
    fn mismatched_identity_sends_nothing() {
        let mut sat = Sat::new(MockDrive::default());
        let e = run(
            &mut sat,
            Some(TestKind::Short),
            Some(&Expected {
                model: "WDC WD80EFAX-68KNBN0".into(),
                serial: "OTHER".into(),
            }),
            Q,
        )
        .unwrap_err();
        assert!(e.to_string().starts_with("refused: the drive is"), "{e}");
        assert!(offline_subcommands(&sat).is_empty());
    }

    #[test]
    fn matching_identity_runs() {
        let mut sat = Sat::new(MockDrive::default());
        let probe = run(&mut sat, None, None, Q).unwrap();
        let out = run(
            &mut sat,
            Some(TestKind::Short),
            Some(&Expected {
                model: probe.model,
                serial: probe.serial,
            }),
            Q,
        )
        .unwrap();
        assert!(out.message.starts_with("Short self-test started"));
        assert!(out.confirmed);
    }

    #[test]
    fn recovered_error_sense_without_err_is_success() {
        let mut sat = Sat::new(MockDrive {
            offline_sense: true,
            ..Default::default()
        });
        assert!(run(&mut sat, Some(TestKind::Short), None, Q).is_ok());
    }

    #[test]
    fn start_identifies_then_sends_the_subcommand() {
        let mut sat = Sat::new(MockDrive::default());
        let out = run(&mut sat, Some(TestKind::Long), None, Q).unwrap();
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
        let out = run(&mut sat, None, None, Q).unwrap();
        assert_eq!(out.message, "Self-test aborted");
        assert_eq!(offline_subcommands(&sat), [0x7f]);
    }

    #[test]
    fn rejected_command_is_an_error() {
        let mut sat = Sat::new(MockDrive {
            offline_rejected: true,
            ..Default::default()
        });
        let e = run(&mut sat, Some(TestKind::Short), None, Q).unwrap_err();
        assert!(
            format!("{e:#}").starts_with("self-test not started"),
            "{e:#}"
        );
    }
}
