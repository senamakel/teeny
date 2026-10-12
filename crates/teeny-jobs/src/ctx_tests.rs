use std::sync::Arc;

use chrono::{TimeDelta, TimeZone, Utc};
use teeny_cadence::clock::ManualClock;
use teeny_store::journal::Outcome;

use super::*;
use crate::runner::ScriptedRunner;

#[test]
fn a_job_keeps_one_guard_across_runs() {
    let ctx = JobCtx::fake(Arc::new(ScriptedRunner::new()));
    let now = Utc.with_ymd_and_hms(2026, 1, 1, 7, 0, 0).unwrap();
    ctx.guard("morning").mark(now);
    assert!(
        ctx.guard("morning")
            .check(now + TimeDelta::hours(1))
            .is_err()
    );
    assert!(ctx.guard("moon").check(now).is_ok());
}

#[test]
fn the_clock_can_be_held_still() {
    let at = Utc.with_ymd_and_hms(2026, 3, 4, 13, 0, 0).unwrap();
    let ctx =
        JobCtx::fake(Arc::new(ScriptedRunner::new())).with_clock(Arc::new(ManualClock::new(at)));
    assert_eq!(ctx.now(), at);
}

#[test]
fn a_declined_report_says_why_and_posted_nothing() {
    let report = JobReport::declined("moon", "not its turn");
    assert_eq!(report.outcome, Outcome::Skipped);
    assert_eq!(report.note.as_deref(), Some("not its turn"));
    assert!(report.posted_id.is_none());
    assert!(!report.ran);
}

#[test]
fn a_job_can_keep_a_guard_with_its_own_interval() {
    let ctx = JobCtx::fake(Arc::new(ScriptedRunner::new()));
    let now = Utc.with_ymd_and_hms(2026, 1, 1, 7, 0, 0).unwrap();
    ctx.guard_with("calendar:gm", TimeDelta::hours(1)).mark(now);
    let guard = ctx.guard_with("calendar:gm", TimeDelta::hours(1));
    assert!(guard.check(now + TimeDelta::minutes(30)).is_err());
    assert!(guard.check(now + TimeDelta::minutes(61)).is_ok());
}
