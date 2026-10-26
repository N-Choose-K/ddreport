use crate::model::SelfTestProgress;
use crate::tui::app::selftest::schedule_after_read;

use super::*;

#[test]
fn test_started_shows_message_and_polls() {
    let mut a = app();
    scanned(&mut a, vec![live("/dev/a")]);
    let t0 = Instant::now();
    loaded(&mut a, "smartctl:/dev/a", with_serial("A1"), t0);
    a.on_key(key('s'));
    let target = start_target(a.on_key(key('y')));
    a.on_msg(
        Msg::TestStarted {
            target,
            kind: TestKind::Short,
            result: Ok("Please wait 2 minutes".into()),
        },
        t0,
    );
    assert!(
        matches!(&a.modal, Some(Modal::Message { body, error: false, .. }) if body.contains("2 minutes"))
    );
    assert!(a.test_cmd_busy.is_empty());

    assert!(a.on_tick(t0 + Duration::from_secs(1)).is_empty());
    let jobs = a.on_tick(t0 + TEST_KICK);
    assert!(matches!(jobs.as_slice(), [Job::Load { .. }]));

    let r = DriveReport {
        self_test_in_progress: Some(SelfTestProgress {
            kind: Some("Short".into()),
            remaining_pct: Some(60),
        }),
        ..Default::default()
    };
    let t1 = t0 + TEST_KICK;
    loaded(&mut a, "smartctl:/dev/a", r, t1);
    assert!(
        a.on_tick(t1 + TEST_POLL - Duration::from_secs(1))
            .is_empty()
    );
    assert_eq!(a.on_tick(t1 + TEST_POLL).len(), 1);

    loaded(&mut a, "smartctl:/dev/a", idle(), t1 + TEST_POLL);
    assert!(
        a.on_tick(t1 + TEST_POLL * 10).is_empty(),
        "polling stops once the test is done"
    );
}

fn started(a: &mut App, now: Instant) {
    a.on_key(key('s'));
    let target = start_target(a.on_key(key('y')));
    a.on_msg(
        Msg::TestStarted {
            target,
            kind: TestKind::Short,
            result: Ok("ok".into()),
        },
        now,
    );
    a.modal = None;
}

fn running() -> DriveReport {
    DriveReport {
        self_test_in_progress: Some(SelfTestProgress {
            kind: Some("Short".into()),
            remaining_pct: Some(90),
        }),
        ..Default::default()
    }
}

const KEY_A: &str = "smartctl:/dev/a";

#[test]
fn polling_survives_failed_reads_after_start() {
    let mut a = app();
    scanned(&mut a, vec![live("/dev/a")]);
    let t0 = Instant::now();
    loaded(&mut a, KEY_A, with_serial("A1"), t0);
    started(&mut a, t0);

    // Reads keep failing: retry with doubling delays capped at TEST_POLL.
    let mut t = t0 + TEST_KICK;
    for secs in [3, 6, 12, 24, 30, 30] {
        assert_eq!(a.on_tick(t).len(), 1, "retry due");
        a.complete_load(KEY_A, Err("bridge reset".into()), t);
        let next = a.devices[0].next_refresh.expect("still polling");
        assert_eq!(next - t, Duration::from_secs(secs));
        t = next;
    }

    // First fresh read shows the test running: normal 30 s polling.
    assert_eq!(a.on_tick(t).len(), 1);
    a.complete_load(KEY_A, Ok(Box::new(running())), t);
    assert!(
        a.devices[0].watch.is_some(),
        "monitored while the test runs"
    );
    assert_eq!(a.devices[0].next_refresh, Some(t + TEST_POLL));
}

#[test]
fn read_from_before_the_command_does_not_stop_polling() {
    let mut a = app();
    scanned(&mut a, vec![live("/dev/a")]);
    let t0 = Instant::now();
    loaded(&mut a, KEY_A, with_serial("A1"), t0);
    a.on_key(key('r'));
    let old_req = a.devices[0].pending.unwrap();
    started(&mut a, t0);
    assert!(a.devices[0].watch.is_some());

    // The pre-command read lands showing no test: keep watching.
    a.complete_load(KEY_A, Ok(Box::new(idle())), t0);
    assert!(old_req < a.devices[0].watch.as_ref().unwrap().first_req);
    assert_eq!(a.devices[0].next_refresh, Some(t0 + TEST_KICK));

    // A fresh read showing no test ends polling.
    a.on_tick(t0 + TEST_KICK);
    a.complete_load(KEY_A, Ok(Box::new(idle())), t0 + TEST_KICK);
    assert!(a.devices[0].watch.is_none());
    assert_eq!(a.devices[0].next_refresh, None);
}

fn start_result(target: TestTarget) -> Msg {
    Msg::TestStarted {
        target,
        kind: TestKind::Short,
        result: Ok("ok".into()),
    }
}

fn confirmed_start(a: &mut App) -> TestTarget {
    a.on_key(key('s'));
    start_target(a.on_key(key('y')))
}

fn rescan_same(a: &mut App, now: Instant) {
    a.on_key(key('R'));
    a.on_msg(Msg::scanned(vec![live("/dev/a")]), now);
    assert!(a.devices[0].loading());
}

#[test]
fn monitoring_survives_rescan_before_command_result() {
    let mut a = app();
    scanned(&mut a, vec![live("/dev/a")]);
    let t0 = Instant::now();
    loaded(&mut a, KEY_A, with_serial("A1"), t0);
    let target = confirmed_start(&mut a);
    rescan_same(&mut a, t0);
    a.on_msg(start_result(target), t0);
    assert_eq!(
        a.devices[0].watch.as_ref().unwrap().serial.as_deref(),
        Some("A1")
    );
    a.complete_load(KEY_A, Err("bridge reset".into()), t0);
    assert_eq!(a.devices[0].next_refresh, Some(t0 + TEST_KICK));
}

#[test]
fn monitoring_survives_rescan_after_command_result() {
    let mut a = app();
    scanned(&mut a, vec![live("/dev/a")]);
    let t0 = Instant::now();
    loaded(&mut a, KEY_A, with_serial("A1"), t0);
    let target = confirmed_start(&mut a);
    a.on_msg(start_result(target), t0);
    a.modal = None;
    rescan_same(&mut a, t0);
    assert!(a.devices[0].watch.is_some(), "carried to the new entry");
    a.complete_load(KEY_A, Err("bridge reset".into()), t0);
    assert_eq!(a.devices[0].next_refresh, Some(t0 + TEST_KICK));
    a.complete_load(KEY_A, Ok(Box::new(running())), t0 + TEST_KICK);
    assert!(a.devices[0].watch.is_some());
    assert_eq!(a.devices[0].next_refresh, Some(t0 + TEST_KICK + TEST_POLL));
}

fn running_on(serial: &str) -> DriveReport {
    let mut r = running();
    r.identity = with_serial(serial).identity;
    r
}

#[test]
fn established_monitoring_survives_rescan_and_failed_read() {
    let mut a = app();
    scanned(&mut a, vec![live("/dev/a")]);
    let t0 = Instant::now();
    loaded(&mut a, KEY_A, with_serial("A1"), t0);
    started(&mut a, t0);
    a.on_tick(t0 + TEST_KICK);
    let t1 = t0 + TEST_KICK;
    a.complete_load(KEY_A, Ok(Box::new(running_on("A1"))), t1);
    assert_eq!(a.devices[0].next_refresh, Some(t1 + TEST_POLL));

    rescan_same(&mut a, t1);
    assert!(a.devices[0].report.is_none(), "new entry has no report yet");
    a.complete_load(KEY_A, Err("bridge reset".into()), t1);
    assert_eq!(
        a.devices[0].next_refresh,
        Some(t1 + TEST_KICK),
        "still scheduled"
    );

    let t2 = t1 + TEST_KICK;
    assert_eq!(a.on_tick(t2).len(), 1);
    a.complete_load(KEY_A, Ok(Box::new(running_on("A1"))), t2);
    assert_eq!(a.devices[0].next_refresh, Some(t2 + TEST_POLL));

    // Test completed: monitoring ends.
    let t3 = t2 + TEST_POLL;
    assert_eq!(a.on_tick(t3).len(), 1);
    a.complete_load(KEY_A, Ok(Box::new(with_serial("A1"))), t3);
    assert!(a.devices[0].watch.is_none());
    assert_eq!(a.devices[0].next_refresh, None);
    assert!(a.on_tick(t3 + TEST_POLL * 4).is_empty());
}

#[test]
fn read_without_self_test_status_keeps_monitoring() {
    let mut a = app();
    scanned(&mut a, vec![live("/dev/a")]);
    let t0 = Instant::now();
    loaded(&mut a, KEY_A, running_on("A1"), t0);
    assert!(a.devices[0].watch.is_some());

    // Partial read: self-test state missing, so it may still be running.
    let t1 = t0 + TEST_POLL;
    assert_eq!(a.on_tick(t1).len(), 1);
    let mut partial = with_serial("A1");
    partial.self_test_status_known = false;
    a.complete_load(KEY_A, Ok(Box::new(partial)), t1);
    assert!(a.devices[0].watch.is_some(), "still monitored");
    assert_eq!(a.devices[0].next_refresh, Some(t1 + TEST_KICK));

    // Known idle: monitoring ends.
    let t2 = t1 + TEST_KICK;
    assert_eq!(a.on_tick(t2).len(), 1);
    a.complete_load(KEY_A, Ok(Box::new(with_serial("A1"))), t2);
    assert!(a.devices[0].watch.is_none());
    assert_eq!(a.devices[0].next_refresh, None);
}

#[test]
fn statusless_drive_stops_monitoring_after_abort() {
    let mut a = app();
    scanned(&mut a, vec![live("/dev/a")]);
    let t0 = Instant::now();
    loaded(&mut a, KEY_A, with_serial("A1"), t0);
    a.on_key(key('a'));
    let target = start_target(a.on_key(key('y')));
    a.on_msg(
        Msg::TestAborted {
            target,
            result: Ok("Self-test aborted".into()),
        },
        t0,
    );
    let mut t = t0 + TEST_KICK;
    for i in 1..=MAX_UNKNOWN_READS {
        assert_eq!(a.on_tick(t).len(), 1, "read {i} scheduled");
        let mut r = with_serial("A1");
        r.self_test_status_known = false;
        a.complete_load(KEY_A, Ok(Box::new(r)), t);
        t = a.devices[0].next_refresh.unwrap_or(t);
    }
    assert!(a.devices[0].watch.is_none());
    assert_eq!(a.devices[0].next_refresh, None);
    assert!(a.on_tick(t + TEST_POLL * 10).is_empty());
    assert_eq!(
        a.status.as_ref().unwrap().text,
        "Drive does not report self-test status; stopped monitoring"
    );
}

#[test]
fn externally_started_test_is_monitored() {
    let mut d = live("/dev/a");
    let now = Instant::now();
    d.report = Some(running_on("A1"));
    assert_eq!(schedule_after_read(&mut d, 7, true, now), None);
    let w = d.watch.clone().expect("monitoring started");
    assert_eq!((w.first_req, w.serial.as_deref()), (7, Some("A1")));
    assert_eq!(d.next_refresh, Some(now + TEST_POLL));

    // Read error while monitored: quick retry.
    assert_eq!(schedule_after_read(&mut d, 8, false, now), None);
    assert_eq!(d.next_refresh, Some(now + TEST_KICK));

    // Fresh read from a different drive at the path ends monitoring.
    d.report = Some(running_on("Z9"));
    let warn = schedule_after_read(&mut d, 9, true, now).unwrap();
    assert!(warn.contains("different drive"));
    assert!(d.watch.is_none() && d.next_refresh.is_none());
}

#[test]
fn late_command_result_skips_a_different_drive() {
    let mut a = app();
    scanned(&mut a, vec![live("/dev/a")]);
    let t0 = Instant::now();
    loaded(&mut a, KEY_A, with_serial("A1"), t0);
    let target = confirmed_start(&mut a);
    rescan_same(&mut a, t0);
    a.complete_load(KEY_A, Ok(Box::new(with_serial("Z9"))), t0);
    a.on_msg(start_result(target), t0);
    assert!(a.devices[0].watch.is_none());
    assert_eq!(a.devices[0].next_refresh, None);
}

#[test]
fn carried_monitoring_stops_when_serial_differs() {
    let mut a = app();
    scanned(&mut a, vec![live("/dev/a")]);
    let t0 = Instant::now();
    loaded(&mut a, KEY_A, with_serial("A1"), t0);
    let target = confirmed_start(&mut a);
    a.on_msg(start_result(target), t0);
    rescan_same(&mut a, t0);
    a.complete_load(KEY_A, Ok(Box::new(with_serial("Z9"))), t0);
    assert!(a.devices[0].watch.is_none());
    assert_eq!(a.devices[0].next_refresh, None);
    assert!(a.status.as_ref().unwrap().text.contains("different drive"));
}
