use crate::tui::app::selftest::still_confirmed;

use super::*;

#[test]
fn confirmation_needs_known_identity() {
    let mut a = app();
    scanned(&mut a, vec![live("/dev/a")]);
    a.on_key(key('s'));
    assert!(a.modal.is_none(), "no report yet");
    loaded(
        &mut a,
        "smartctl:/dev/a",
        DriveReport::default(),
        Instant::now(),
    );
    a.on_key(key('s'));
    assert!(a.modal.is_none());
    assert_eq!(
        a.status.as_ref().unwrap().text,
        "Serial unknown - re-read the drive first"
    );
}

#[test]
fn still_confirmed_compares_every_field() {
    let t = TestTarget {
        key: "smartctl:/dev/a".into(),
        label: "/dev/a".into(),
        via: TestVia::Smartctl {
            path: "/dev/a".into(),
            dev_type: Some("sat".into()),
        },
        generation: 2,
        model: Some("M".into()),
        serial: Some("S".into()),
    };
    assert!(still_confirmed(&t, Some(&t.clone())));
    assert!(!still_confirmed(&t, None));
    let changed: [fn(&mut TestTarget); 5] = [
        |t| {
            t.via = TestVia::Smartctl {
                path: "/dev/b".into(),
                dev_type: Some("sat".into()),
            }
        },
        |t| {
            t.via = TestVia::Smartctl {
                path: "/dev/a".into(),
                dev_type: Some("nvme".into()),
            }
        },
        |t| t.generation = 3,
        |t| t.model = Some("Other".into()),
        |t| t.serial = Some("S2".into()),
    ];
    for change in changed {
        let mut c = t.clone();
        change(&mut c);
        assert!(!still_confirmed(&t, Some(&c)), "{c:?}");
    }
    let unknown = TestTarget {
        serial: None,
        ..t.clone()
    };
    assert!(!still_confirmed(&unknown, Some(&unknown.clone())));
}

#[test]
fn confirmation_cancelled_when_drive_list_changes() {
    let mut a = app();
    scanned(&mut a, vec![live("/dev/a"), live("/dev/b")]);
    loaded(&mut a, "smartctl:/dev/a", with_serial("A1"), Instant::now());
    loaded(&mut a, "smartctl:/dev/b", with_serial("B1"), Instant::now());
    a.on_key(key('j'));
    // R then s: the scan lands while the confirmation is open.
    a.on_key(key('R'));
    a.on_key(key('s'));
    let Some(Modal::Confirm(_, t)) = &a.modal else {
        panic!("expected confirmation");
    };
    assert_eq!(t.label, "/dev/b");
    assert_eq!(t.serial.as_deref(), Some("B1"));
    a.on_msg(
        Msg::scanned(vec![live("/dev/b"), live("/dev/a")]),
        Instant::now(),
    );
    assert!(a.modal.is_none());
    assert!(a.status.as_ref().unwrap().error);
    assert!(a.on_key(key('y')).is_empty(), "nothing left to confirm");
}

#[test]
fn confirmation_cancelled_when_serial_changes() {
    let mut a = app();
    scanned(&mut a, vec![live("/dev/a")]);
    loaded(&mut a, "smartctl:/dev/a", with_serial("A1"), Instant::now());
    a.on_key(key('l'));
    // A refresh reveals a different drive behind the same path.
    loaded(&mut a, "smartctl:/dev/a", with_serial("Z9"), Instant::now());
    assert!(a.on_key(key('y')).is_empty());
    assert!(a.test_cmd_busy.is_empty());
    assert_eq!(
        a.status.as_ref().unwrap().text,
        "Cancelled: the drive changed since confirmation"
    );

    a.on_key(key('l'));
    let target = start_target(a.on_key(key('y')));
    assert_eq!(target.serial.as_deref(), Some("Z9"));
    assert_eq!(target.generation, a.generation);
}

#[test]
fn self_test_needs_confirmation() {
    let mut a = app();
    scanned(&mut a, vec![live("/dev/a")]);
    let mut r = with_serial("A1");
    r.meta.device_type = "sntjmicron".into();
    loaded(&mut a, "smartctl:/dev/a", r, Instant::now());

    assert!(a.on_key(key('s')).is_empty());
    assert!(matches!(
        a.modal,
        Some(Modal::Confirm(Confirm::Start(TestKind::Short), _))
    ));
    assert!(a.on_key(key('n')).is_empty());
    assert!(a.modal.is_none());

    a.on_key(key('l'));
    let jobs = a.on_key(key('y'));
    let [Job::StartTest { target, kind }] = jobs.as_slice() else {
        panic!("expected StartTest, got {jobs:?}");
    };
    assert_eq!(
        target.via,
        TestVia::Smartctl {
            path: "/dev/a".into(),
            dev_type: Some("sntjmicron".into()),
        }
    );
    assert_eq!(*kind, TestKind::Long);

    a.on_key(key('a'));
    assert!(a.modal.is_none(), "blocked while a command is in flight");
    assert!(a.status.as_ref().unwrap().error);
}

#[test]
fn self_test_refused_for_demo_files() {
    let mut a = app();
    scanned(
        &mut a,
        vec![Device::new(DeviceSource::DemoFile("x.json".into()), "")],
    );
    a.on_key(key('s'));
    assert!(a.modal.is_none());
    assert!(a.status.as_ref().unwrap().error);
}

fn usb(mounts: &[&str]) -> Device {
    Device::new(
        DeviceSource::NativeUsb {
            selector: "1f75:0611".parse().unwrap(),
            bridge: "Hub".into(),
            disk: Some(crate::native::UsbDisk {
                bsd: "disk4".into(),
                media_name: Some("CT1050MX300SSD1".into()),
                size_bytes: Some(1_050_214_588_416),
                mounts: mounts.iter().map(|m| m.to_string()).collect(),
            }),
        },
        "USB disk4 1.05 TB",
    )
}

#[test]
fn remount_confirms_then_loads_with_remount() {
    let mut a = app();
    let jobs = scanned(&mut a, vec![live("/dev/a"), usb(&["/Volumes/macOS"])]);
    assert_eq!(jobs.len(), 2, "USB entries load on scan too");
    assert_eq!(a.devices[1].source.label(), "CT1050MX300SSD1");

    a.on_key(key('m'));
    assert!(a.modal.is_none(), "smartctl drive selected");
    assert!(a.status.as_ref().unwrap().error);

    a.on_key(code(KeyCode::Down));
    a.on_key(key('m'));
    assert!(a.status.as_ref().unwrap().error, "read still in flight");
    a.complete_load(
        "usb:1f75:0611",
        Err("its volumes are mounted".into()),
        Instant::now(),
    );
    a.on_key(key('m'));
    let Some(Modal::Remount { key: k, mounts, .. }) = &a.modal else {
        panic!("expected remount modal, got {:?}", a.modal);
    };
    assert_eq!(k, "usb:1f75:0611");
    assert_eq!(mounts, &["/Volumes/macOS"]);

    a.on_key(key('n'));
    assert!(a.modal.is_none());
    a.on_key(key('m'));
    let jobs = a.on_key(key('y'));
    assert!(
        matches!(jobs.as_slice(), [Job::Load { remount: true, .. }]),
        "{jobs:?}"
    );
    assert!(a.devices[1].loading());
}

#[test]
fn rescan_cancels_remount_modal() {
    let mut a = app();
    scanned(&mut a, vec![usb(&[])]);
    a.complete_load("usb:1f75:0611", Err("x".into()), Instant::now());
    a.on_key(key('m'));
    assert!(matches!(a.modal, Some(Modal::Remount { .. })));
    a.on_key(key('R'));
    a.on_msg(Msg::scanned(vec![usb(&[])]), Instant::now());
    assert!(a.modal.is_none());
}

#[test]
fn usb_drive_confirms_and_polls_with_remount() {
    let mut a = app();
    scanned(&mut a, vec![usb(&["/Volumes/x"])]);
    let t0 = Instant::now();
    loaded(&mut a, "usb:1f75:0611", with_serial("S9"), t0);
    a.on_key(key('s'));
    let Some(Modal::Confirm(_, t)) = &a.modal else {
        panic!("expected confirmation, got {:?}", a.modal);
    };
    assert_eq!(t.label, "CT1050MX300SSD1");
    assert!(matches!(t.via, TestVia::NativeUsb { .. }));
    let target = start_target(a.on_key(key('y')));
    a.on_msg(
        Msg::TestStarted {
            target: target.clone(),
            kind: TestKind::Short,
            result: Ok("Short self-test started".into()),
        },
        t0,
    );
    let jobs = a.on_tick(t0 + TEST_KICK);
    assert!(
        matches!(jobs.as_slice(), [Job::Load { remount: true, .. }]),
        "progress reads remount: {jobs:?}"
    );
}

#[test]
fn unconfirmed_abort_message_is_shown() {
    let mut a = app();
    scanned(&mut a, vec![usb(&[])]);
    let t0 = Instant::now();
    loaded(&mut a, "usb:1f75:0611", with_serial("S9"), t0);
    a.on_key(key('a'));
    let target = start_target(a.on_key(key('y')));
    a.on_msg(
        Msg::TestAborted {
            target,
            result: Ok(
                "Abort sent; this bridge never acknowledges, the next read will confirm".into(),
            ),
        },
        t0,
    );
    let s = a.status.as_ref().unwrap();
    assert!(!s.error);
    assert!(s.text.starts_with("Abort sent"), "{}", s.text);
    assert!(a.devices[0].watch.is_some(), "monitoring confirms it");
}
