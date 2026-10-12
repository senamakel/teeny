use super::*;

#[test]
fn a_manual_clock_stands_still_until_moved() {
    let start = at("2026-09-14T07:00:00Z");
    let clock = ManualClock::new(start);
    assert_eq!(clock.now(), start);
    clock.advance(TimeDelta::hours(2));
    assert_eq!(clock.now(), at("2026-09-14T09:00:00Z"));
    clock.set(start);
    assert_eq!(clock.now(), start);
}

#[test]
fn the_system_clock_reads_the_time() {
    let before = Utc::now();
    assert!(SystemClock.now() >= before);
}

#[test]
fn writes_timestamps_as_javascript_did() {
    assert_eq!(iso(at("2026-09-14T07:00:00Z")), "2026-09-14T07:00:00.000Z");
}
