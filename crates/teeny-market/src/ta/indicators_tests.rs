use serde::Deserialize;

use super::*;

fn series(closes: &[f64]) -> Vec<Candle> {
    closes
        .iter()
        .enumerate()
        .map(|(index, &close)| Candle {
            time: 1_700_000_000 + index as i64 * 3_600,
            open: if index == 0 { close } else { closes[index - 1] },
            high: close * 1.01,
            low: close * 0.99,
            close,
            volume: 100.0,
        })
        .collect()
}

#[test]
fn has_no_value_until_it_has_enough_history() {
    let values = sma(&[1.0, 2.0, 3.0, 4.0, 5.0], 3);
    assert_eq!(&values[..2], &[None, None]);
    assert_eq!(values[2], Some(2.0));
    assert_eq!(values[4], Some(4.0));
}

#[test]
fn seeds_the_ema_on_the_simple_average_not_on_the_first_close() {
    assert_eq!(ema(&[1.0, 2.0, 3.0, 4.0, 5.0], 3)[2], Some(2.0));
}

#[test]
fn returns_nothing_at_all_when_the_series_is_shorter_than_the_period() {
    assert!(ema(&[1.0, 2.0], 5).iter().all(Option::is_none));
}

#[test]
fn brackets_the_price_and_widens_with_volatility() {
    let calm = bollinger(&[100.0; 40], 20, 2.0);
    assert!((latest(&calm.upper).unwrap() - 100.0).abs() < 1e-6);
    assert!((latest(&calm.lower).unwrap() - 100.0).abs() < 1e-6);
    let choppy: Vec<f64> = (0..40)
        .map(|i| if i % 2 == 0 { 90.0 } else { 110.0 })
        .collect();
    let choppy = bollinger(&choppy, 20, 2.0);
    assert!(latest(&choppy.upper).unwrap() > latest(&calm.upper).unwrap());
}

#[test]
fn measures_true_range_against_the_previous_close() {
    let closes: Vec<f64> = (10..=25).map(f64::from).collect();
    assert!(latest(&atr(&series(&closes), 14)).unwrap() > 0.0);
}

#[test]
fn weights_vwap_by_volume() {
    let mut candles = series(&[10.0, 20.0]);
    candles[0].volume = 999.0;
    candles[1].volume = 1.0;
    assert!(vwap(&candles).unwrap() < 11.0);
}

#[test]
fn will_not_call_a_swing_on_the_last_bars_because_nothing_has_confirmed_them() {
    let candles = series(&[1.0, 2.0, 3.0, 2.0, 1.0, 2.0, 3.0, 4.0, 5.0]);
    assert!(
        swings(&candles, 2)
            .iter()
            .all(|point| point.index < candles.len() - 2)
    );
}

#[test]
fn clusters_repeated_touches_into_one_level() {
    let candles = series(&[
        10.0, 12.0, 10.0, 12.05, 10.0, 11.98, 10.0, 12.02, 10.0, 12.0, 10.0,
    ]);
    assert!(levels(&candles, 2, 0.02)[0].touches > 1);
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Golden {
    name: String,
    candles: Vec<Candle>,
    sma20: Series,
    ema20: Series,
    ema50: Series,
    bollinger: Bands,
    atr14: Series,
    vwap: Option<f64>,
    swings: Vec<Swing>,
    levels: Vec<Level>,
    levels_loose: Vec<Level>,
}

/// Bit-for-bit what the TypeScript computed on the same candles.
#[test]
fn matches_the_typescript_exactly() {
    let cases: Vec<Golden> =
        serde_json::from_str(include_str!("../../fixtures/golden/indicators.json")).unwrap();
    for case in cases {
        let closes = closes(&case.candles);
        let name = &case.name;
        assert_eq!(sma(&closes, 20), case.sma20, "{name} sma20");
        assert_eq!(ema(&closes, 20), case.ema20, "{name} ema20");
        assert_eq!(ema(&closes, 50), case.ema50, "{name} ema50");
        assert_eq!(
            bollinger(&closes, 20, 2.0),
            case.bollinger,
            "{name} bollinger"
        );
        assert_eq!(atr(&case.candles, 14), case.atr14, "{name} atr");
        assert_eq!(vwap(&case.candles), case.vwap, "{name} vwap");
        assert_eq!(swings(&case.candles, 2), case.swings, "{name} swings");
        assert_eq!(levels_default(&case.candles), case.levels, "{name} levels");
        assert_eq!(
            levels(&case.candles, 3, 0.02),
            case.levels_loose,
            "{name} loose levels"
        );
    }
}
