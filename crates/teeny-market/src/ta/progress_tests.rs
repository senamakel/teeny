use serde::Deserialize;
use serde_json::Value;

use super::*;
use crate::test_support::{assert_same_json, golden};

fn daily(closes: &[f64]) -> Vec<Candle> {
    closes
        .iter()
        .enumerate()
        .map(|(index, &close)| {
            let open = if index == 0 { close } else { closes[index - 1] };
            Candle {
                time: 1_700_000_000 + index as i64 * 86_400,
                open,
                high: open.max(close) * 1.02,
                low: open.min(close) * 0.98,
                close,
                volume: 1_000.0,
            }
        })
        .collect()
}

#[test]
fn measures_the_horizons_it_has_the_history_for() {
    let closes: Vec<f64> = (0..100).map(|i| 1.0 + f64::from(i) * 0.1).collect();
    let progress = read_progress(&daily(&closes)).unwrap();
    assert_eq!(
        progress.changes.iter().map(|c| c.days).collect::<Vec<_>>(),
        vec![7, 30, 90]
    );
    assert!(progress.changes[0].change_pct > 0.0);
}

/// A token three weeks old still has a story; it just is not a 90-day one.
#[test]
fn falls_back_to_whatever_history_exists_when_the_token_is_young() {
    let progress = read_progress(&daily(&[1.0, 1.2, 1.4, 1.8])).unwrap();
    assert_eq!(
        progress.changes,
        vec![HorizonChange {
            days: 3,
            change_pct: 80.0
        }]
    );
}

#[test]
fn counts_green_days_and_finds_stepping_lows() {
    let closes = [
        20.0, 14.0, 20.0, 26.0, 18.0, 26.0, 32.0, 24.0, 32.0, 38.0, 30.0, 38.0,
    ];
    let progress = read_progress(&daily(&closes)).unwrap();
    assert!(progress.green_days > 0);
    assert!(progress.higher_lows);
}

#[test]
fn says_plainly_when_the_lows_are_not_stepping_up() {
    let progress = read_progress(&daily(&[
        20.0, 18.0, 19.0, 15.0, 16.0, 12.0, 13.0, 9.0, 10.0,
    ]))
    .unwrap();
    assert!(!progress.higher_lows);
    assert!(progress.notes.join(" ").contains("not yet stepping up"));
}

#[test]
fn finds_the_positive_on_the_long_horizon_which_is_where_it_lives_on_a_red_day() {
    let progress = read_progress(&daily(&[1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 30.0, 28.0])).unwrap();
    assert!(!progress.positives.is_empty());
    assert!(
        progress
            .notes
            .join(" ")
            .contains("TRUE POSITIVES YOU CAN LEAD WITH")
    );
}

#[test]
fn still_has_one_when_every_horizon_is_down() {
    let closes: Vec<f64> = (0..40).map(|i| 100.0 - f64::from(i) * 2.0).collect();
    let progress = read_progress(&daily(&closes)).unwrap();
    assert!(!progress.positives.is_empty());
    assert!(
        progress
            .positives
            .iter()
            .all(|entry| !entry.contains("up 0"))
    );
}

#[test]
fn refuses_to_read_progress_from_a_single_day() {
    let error = read_progress(&daily(&[1.0])).unwrap_err();
    assert!(error.to_string().contains("at least 2"));
}

#[test]
fn places_price_against_its_own_daily_range() {
    let progress = read_progress(&daily(&[10.0, 20.0, 15.0])).unwrap();
    assert_eq!((progress.high, progress.low), (20.0, 10.0));
    assert!(progress.from_high_pct < 0.0);
    assert!(progress.from_low_pct > 0.0);
}

#[derive(Deserialize)]
struct Golden {
    name: String,
    candles: Vec<Candle>,
    progress: Value,
}

#[test]
fn matches_the_typescript_exactly() {
    for case in golden::<Vec<Golden>>("progress") {
        let progress = read_progress(&case.candles).unwrap();
        assert_same_json(&progress, &case.progress, &case.name);
    }
}
