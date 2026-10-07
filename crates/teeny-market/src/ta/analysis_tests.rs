use serde::Deserialize;
use serde_json::Value;

use super::*;
use crate::test_support::{assert_same_json, golden};

fn candles(closes: &[f64], volumes: Option<&[f64]>) -> Vec<Candle> {
    closes
        .iter()
        .enumerate()
        .map(|(index, &close)| {
            let open = if index == 0 { close } else { closes[index - 1] };
            Candle {
                time: 1_700_000_000 + index as i64 * 3_600,
                open,
                high: open.max(close) * 1.005,
                low: open.min(close) * 0.995,
                close,
                volume: volumes.map_or(100.0, |v| v[index]),
            }
        })
        .collect()
}

fn rising() -> Vec<f64> {
    (0..120).map(|i| 100.0 + f64::from(i) * 1.5).collect()
}

fn falling() -> Vec<f64> {
    (0..120).map(|i| 300.0 - f64::from(i) * 1.5).collect()
}

fn read(closes: &[f64]) -> TechnicalRead {
    analyse(&candles(closes, None), Timeframe::H1).unwrap()
}

fn read_with(closes: &[f64], volumes: &[f64]) -> TechnicalRead {
    analyse(&candles(closes, Some(volumes)), Timeframe::H1).unwrap()
}

fn has_signal(read: &TechnicalRead, name: &str, bullish: Option<bool>) -> bool {
    read.signals
        .iter()
        .any(|s| s.name == name && bullish.is_none_or(|b| s.bullish == b))
}

#[test]
fn refuses_to_analyse_a_series_it_cannot_say_anything_about() {
    let error = analyse(&candles(&[1.0, 2.0], None), Timeframe::H1).unwrap_err();
    assert!(error.to_string().contains("at least 3"));
}

#[test]
fn reads_a_clean_uptrend_as_bullish() {
    let read = read(&rising());
    assert_eq!(read.trend, Trend::Up);
    assert!(read.change_pct > 0.0);
    assert!(read.bullish_score > 60);
}

/// The regression that mattered: a token down a third over five days scored
/// bullish on a two-hour bounce.
#[test]
fn does_not_score_a_bounce_inside_a_downtrend_as_bullish() {
    let mut bounce = falling()[..112].to_vec();
    bounce.extend([140.0, 143.0, 147.0, 152.0, 158.0, 165.0, 173.0, 182.0]);
    let read = read(&bounce);
    assert!(read.change_pct < 0.0);
    assert!(has_signal(&read, "window trend", Some(false)));
    assert!(read.bullish_score < 80);
}

#[test]
fn scores_a_sustained_downtrend_as_weak() {
    let read = read(&falling());
    assert_eq!(read.trend, Trend::Down);
    assert!(read.bullish_score < 40);
}

#[test]
fn reports_levels_only_where_they_exist_and_keeps_them_on_the_right_side_of_price() {
    let read = read(&rising());
    if let Some(support) = read.levels.support {
        assert!(support < read.price);
    }
    if let Some(resistance) = read.levels.resistance {
        assert!(resistance > read.price);
    }
    assert!(read.levels.window_high >= read.levels.window_low);
}

#[test]
fn notices_a_volume_spike_on_the_last_bar() {
    let volumes: Vec<f64> = (0..120)
        .map(|i| if i == 119 { 5_000.0 } else { 100.0 })
        .collect();
    let read = read_with(&rising(), &volumes);
    assert!(has_signal(&read, "volume spike", None));
    assert!(read.volume.ratio > 1.5);
}

#[test]
fn notices_volume_waking_up_across_the_window_not_just_on_one_bar() {
    let volumes: Vec<f64> = (0..120)
        .map(|i| if i < 60 { 100.0 } else { 400.0 })
        .collect();
    let read = read_with(&rising(), &volumes);
    assert!((read.volume.trend - 4.0).abs() < 0.05);
    assert!(has_signal(&read, "volume trend", Some(true)));
}

#[test]
fn notices_volume_drying_up() {
    let volumes: Vec<f64> = (0..120)
        .map(|i| if i < 60 { 400.0 } else { 100.0 })
        .collect();
    assert!(has_signal(
        &read_with(&rising(), &volumes),
        "volume trend",
        Some(false)
    ));
}

/// The prompt forbids "overbought"; a tool that reports it is an invitation.
#[test]
fn computes_no_oscillators_at_all() {
    let surface = serde_json::to_string(&read(&rising()))
        .unwrap()
        .to_lowercase();
    for word in ["rsi", "macd", "overbought", "oversold"] {
        assert!(!surface.contains(word), "{word}");
    }
}

#[test]
fn writes_notes_that_quote_only_numbers_it_computed() {
    let read = read(&rising());
    assert!(
        read.notes
            .join(" ")
            .contains(&format!("{}/100", read.bullish_score))
    );
    assert!(read.notes.iter().any(|note| note.contains("bars")));
}

/// Four visits to ~100 that get rejected, then a close through it.
fn with_level() -> Vec<f64> {
    let mut closes: Vec<f64> = (0..20).map(|i| 90.0 + f64::from(i) * 0.2).collect();
    closes.extend([100.0, 94.0, 100.0, 93.0, 99.8, 92.0, 100.2, 95.0]);
    closes.extend((0..12).map(|i| 96.0 + f64::from(i) * 0.6));
    closes
}

#[test]
fn calls_a_break_above_a_defended_level_and_says_whether_volume_backed_it() {
    let closes = with_level();
    let volumes: Vec<f64> = (0..closes.len())
        .map(|i| if i >= closes.len() - 3 { 900.0 } else { 100.0 })
        .collect();
    let read = read_with(&closes, &volumes);
    let event = read.event.expect("an event");
    assert!(event.touches >= 2);
    if event.kind != LevelEventKind::Breakdown {
        assert!(has_signal(&read, event.kind.as_str(), None));
    }
}

#[test]
fn downgrades_a_break_that_volume_did_not_confirm() {
    let quiet = read(&with_level());
    if let Some(event) = quiet.event {
        assert!(!event.volume_confirmed);
        let signal = quiet
            .signals
            .iter()
            .find(|s| s.name == event.kind.as_str())
            .unwrap();
        assert_eq!(signal.strength, Strength::Moderate);
        assert!(signal.detail.contains("volume did not confirm"));
    }
}

#[test]
fn reports_no_event_on_a_series_that_never_crosses_anything_twice_touched() {
    assert_eq!(read(&rising()).event, None);
}

#[test]
fn always_finds_a_positive_even_on_a_chart_that_has_done_nothing_but_fall() {
    let read = read(&falling());
    assert!(read.positives.last().unwrap().contains("up off the low"));
}

#[test]
fn puts_a_defended_level_ahead_of_a_percentage_because_it_is_the_better_sentence() {
    let mut closes: Vec<f64> = (0..20).map(|i| 96.0 + f64::from(i) * 0.2).collect();
    closes.extend([100.0, 94.0, 100.1, 93.9, 99.9, 94.1, 100.0, 94.0]);
    closes.extend((0..20).map(|i| 101.0 + f64::from(i) * 0.5));
    let read = read(&closes);
    let support = read
        .positives
        .iter()
        .position(|e| e.contains("defended"))
        .unwrap();
    let percentage = read
        .positives
        .iter()
        .position(|e| e.contains("across the window"))
        .unwrap();
    assert!(support < percentage);
}

#[test]
fn states_only_things_the_same_read_already_computed() {
    let read = read(&rising());
    if read
        .positives
        .iter()
        .any(|e| e.contains("across the window"))
    {
        assert!(read.change_pct > 0.0);
    }
    for entry in &read.positives {
        let lower = entry.to_lowercase();
        for word in ["will", "should", "target", "soon"] {
            assert!(!lower.contains(word), "{entry}");
        }
    }
}

#[test]
fn offers_the_positives_in_the_notes_labelled_as_the_thing_to_lead_with() {
    assert!(
        read(&falling())
            .notes
            .join(" ")
            .contains("TRUE POSITIVES YOU CAN LEAD WITH")
    );
}

#[test]
fn does_not_claim_volume_is_picking_up_when_it_is_drying_out() {
    let volumes: Vec<f64> = (0..120)
        .map(|i| if i < 60 { 400.0 } else { 100.0 })
        .collect();
    let read = read_with(&rising(), &volumes);
    assert!(!read.positives.iter().any(|e| e.contains("volume is")));
}

fn broken() -> Vec<f64> {
    let mut closes: Vec<f64> = (0..20).map(|i| 100.0 + f64::from(i) * 0.1).collect();
    closes.extend([102.0, 98.0, 102.0, 97.5, 101.8, 98.0, 102.0, 97.0]);
    closes.extend([92.0, 90.5, 91.0, 90.8, 91.2, 91.0, 91.4, 91.1, 91.6]);
    closes
}

#[test]
fn reports_a_new_base_once_it_has_actually_held() {
    assert!(
        read(&broken())
            .positives
            .iter()
            .any(|e| e.contains("new base is forming"))
    );
}

#[test]
fn will_not_call_a_base_on_the_way_down() {
    let mut falling = broken()[..28].to_vec();
    falling.extend([92.0, 90.0, 88.0, 86.0, 84.0, 82.0, 80.0]);
    assert!(
        !read(&falling)
            .positives
            .iter()
            .any(|e| e.contains("new base"))
    );
}

#[test]
fn will_not_call_a_base_on_the_bar_the_level_broke() {
    let mut just_broke = broken()[..28].to_vec();
    just_broke.push(92.0);
    assert!(
        !read(&just_broke)
            .positives
            .iter()
            .any(|e| e.contains("new base"))
    );
}

#[derive(Deserialize)]
struct Golden {
    name: String,
    candles: Vec<Candle>,
    read: Value,
}

/// The whole read -- every number, every sentence -- as the TypeScript wrote it.
#[test]
fn matches_the_typescript_exactly() {
    for case in golden::<Vec<Golden>>("analysis") {
        let read = analyse(&case.candles, Timeframe::H1).unwrap();
        assert_same_json(&read, &case.read, &case.name);
    }
}
