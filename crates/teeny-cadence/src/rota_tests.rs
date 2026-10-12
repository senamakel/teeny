//! Port of "the rota" in `legacy/test/moon.test.ts` (the rota half; the
//! moon and chart jobs are ported with the pipelines).

use super::*;
use crate::clock::at;

fn day(date: &str) -> DateTime<Utc> {
    at(&format!("{date}T20:00:00Z"))
}

#[test]
fn counts_slots_from_the_anchor_and_wraps_across_a_month_end() {
    let rota = Rota::parse(3, "2026-09-14").unwrap();
    assert_eq!(rota.slot(day("2026-09-14")), 0);
    assert_eq!(rota.slot(day("2026-09-15")), 1);
    assert_eq!(rota.slot(day("2026-09-16")), 2);
    assert_eq!(rota.slot(day("2026-09-17")), 0);
    assert_eq!(rota.slot(day("2026-09-30")), 1);
    assert_eq!(rota.slot(day("2026-10-01")), 2);
    assert_eq!(rota.slot(day("2026-10-02")), 0);
    // Before the anchor still lands on a real slot.
    assert_eq!(rota.slot(day("2026-09-13")), 2);
    assert!(rota.is_turn(1, day("2026-09-15")));
    assert!(!rota.is_turn(5, day("2026-09-15")));
    let error = Rota::parse(3, "yesterday").unwrap_err();
    assert!(error.to_string().contains("ISO date"), "{error}");
}

#[test]
fn counts_whole_utc_days_so_the_answer_is_the_same_at_any_hour() {
    let rota = Rota::parse(7, "2026-09-14").unwrap();
    assert_eq!(rota.slot(at("2026-09-14T00:00:00Z")), 0);
    assert_eq!(rota.slot(at("2026-09-14T23:59:59Z")), 0);
    assert_eq!(rota.slot(at("2026-09-15T00:00:00Z")), 1);
}

#[test]
fn reads_the_rota_from_the_configuration_and_refuses_an_empty_one() {
    let rota = Rota::new(7, NaiveDate::from_ymd_opt(2026, 9, 14).unwrap()).unwrap();
    assert_eq!(rota.days(), 7);
    assert!(Rota::parse(0, "2026-09-14").is_err());
}
