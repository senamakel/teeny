//! Port of the `MediaBudget` block of `legacy/test/media.test.ts`.

use chrono::TimeDelta;

use super::*;
use crate::clock::{ManualClock, SystemClock, at};

#[test]
fn counts_images_and_videos_apart_and_starts_over_at_utc_midnight() {
    let clock = Arc::new(ManualClock::new(at("2026-09-14T23:59:00Z")));
    let budget = DailyMediaBudget::new(
        MediaLimits {
            images_per_day: 2,
            videos_per_day: 1,
        },
        clock.clone(),
    );

    assert!(budget.take(MediaKind::Image));
    assert!(budget.take(MediaKind::Image));
    assert!(!budget.take(MediaKind::Image));
    assert_eq!(budget.remaining(MediaKind::Image), 0);
    assert!(budget.take(MediaKind::Video));
    assert!(!budget.take(MediaKind::Video));

    clock.advance(TimeDelta::minutes(2));
    assert_eq!(budget.remaining(MediaKind::Image), 2);
    assert!(budget.take(MediaKind::Video));
}

#[test]
fn is_closed_at_zero() {
    let budget = DailyMediaBudget::new(MediaLimits::default(), Arc::new(SystemClock));
    assert!(!budget.take(MediaKind::Image));
}
