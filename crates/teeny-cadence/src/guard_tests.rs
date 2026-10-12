//! The re-run guard every daily posting job keeps (the legacy jobs'
//! `lastRunAt` against `minIntervalMs`, twenty hours by default).

use super::*;
use crate::clock::at;

#[test]
fn lets_the_first_run_through_and_refuses_another_inside_the_interval() {
    let guard = RerunGuard::default();
    let first = at("2026-09-14T13:00:00Z");
    assert_eq!(guard.check(first), Ok(()));
    guard.mark(first);
    let soon = first + TimeDelta::hours(19);
    assert_eq!(
        guard.check(soon),
        Err(RanRecently {
            since: TimeDelta::hours(19),
            minimum: TimeDelta::hours(20)
        })
    );
    assert_eq!(guard.check(first + TimeDelta::hours(20)), Ok(()));
}

#[test]
fn takes_its_own_interval() {
    let guard = RerunGuard::new(TimeDelta::minutes(60));
    let first = at("2026-09-14T13:00:00Z");
    guard.mark(first);
    assert!(guard.check(first + TimeDelta::minutes(59)).is_err());
    assert!(guard.check(first + TimeDelta::minutes(60)).is_ok());
    assert_eq!(DEFAULT_RERUN_INTERVAL, TimeDelta::hours(20));
}
