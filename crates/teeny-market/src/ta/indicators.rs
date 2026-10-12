//! The indicator kit.
//!
//! Every series function returns a vector the same length as its input, with
//! `None` for the bars before the indicator has enough history. An EMA(50) over
//! 40 bars is not zero and it is not the first close, it is unknown, and a
//! chart that draws it anyway invents a trend line out of the warm-up period.
//!
//! There is no RSI and no MACD here, on purpose: they are mean-reversion
//! tools, and a memecoin does not revert. What is kept describes where price
//! has been: averages, range, volatility, and the levels it keeps returning to.

use serde::{Deserialize, Serialize};

use crate::types::Candle;

/// A series with gaps where there was not enough history.
pub type Series = Vec<Option<f64>>;

/// The closes.
pub fn closes(candles: &[Candle]) -> Vec<f64> {
    candles.iter().map(|candle| candle.close).collect()
}

/// Simple moving average. Panics on a zero period.
pub fn sma(values: &[f64], period: usize) -> Series {
    assert!(period > 0, "period must be positive");
    let mut out = Vec::with_capacity(values.len());
    let mut total = 0.0;
    for (index, value) in values.iter().enumerate() {
        total += value;
        if index >= period {
            total -= values[index - period];
        }
        out.push((index + 1 >= period).then_some(total / period as f64));
    }
    out
}

/// Exponential moving average, seeded with the SMA of the first `period`
/// values rather than with the first value, which is what every charting
/// package does. Panics on a zero period.
pub fn ema(values: &[f64], period: usize) -> Series {
    assert!(period > 0, "period must be positive");
    let mut out = vec![None; values.len()];
    if values.len() < period {
        return out;
    }
    let k = 2.0 / (period as f64 + 1.0);
    let mut previous = values[..period]
        .iter()
        .fold(0.0, |total, value| total + value)
        / period as f64;
    out[period - 1] = Some(previous);
    for index in period..values.len() {
        previous = values[index] * k + previous * (1.0 - k);
        out[index] = Some(previous);
    }
    out
}

/// Bollinger bands.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bands {
    pub upper: Series,
    pub middle: Series,
    pub lower: Series,
}

/// Bollinger bands: the SMA and `deviations` population standard deviations
/// either side of it.
pub fn bollinger(values: &[f64], period: usize, deviations: f64) -> Bands {
    let middle = sma(values, period);
    let mut upper = Vec::with_capacity(values.len());
    let mut lower = Vec::with_capacity(values.len());
    for (index, mean) in middle.iter().enumerate() {
        let Some(mean) = *mean else {
            upper.push(None);
            lower.push(None);
            continue;
        };
        let window = &values[index + 1 - period..=index];
        let variance = window
            .iter()
            .fold(0.0, |total, value| total + (value - mean) * (value - mean))
            / window.len() as f64;
        let sigma = variance.sqrt();
        upper.push(Some(mean + sigma * deviations));
        lower.push(Some(mean - sigma * deviations));
    }
    Bands {
        upper,
        middle,
        lower,
    }
}

/// Average true range, Wilder-smoothed. A volatility scale, not a stop.
pub fn atr(candles: &[Candle], period: usize) -> Series {
    let mut out = vec![None; candles.len()];
    if candles.len() <= period {
        return out;
    }
    let ranges: Vec<f64> = candles
        .iter()
        .enumerate()
        .map(
            |(index, candle)| match index.checked_sub(1).map(|i| candles[i]) {
                None => candle.high - candle.low,
                Some(previous) => (candle.high - candle.low)
                    .max((candle.high - previous.close).abs())
                    .max((candle.low - previous.close).abs()),
            },
        )
        .collect();
    let mut average = ranges[1..=period]
        .iter()
        .fold(0.0, |total, value| total + value)
        / period as f64;
    out[period] = Some(average);
    for index in period + 1..candles.len() {
        average = (average * (period as f64 - 1.0) + ranges[index]) / period as f64;
        out[index] = Some(average);
    }
    out
}

/// Volume-weighted average price over the whole series.
pub fn vwap(candles: &[Candle]) -> Option<f64> {
    let volume = candles
        .iter()
        .fold(0.0, |total, candle| total + candle.volume);
    if volume <= 0.0 {
        return None;
    }
    let weighted = candles.iter().fold(0.0, |total, candle| {
        total + ((candle.high + candle.low + candle.close) / 3.0) * candle.volume
    });
    Some(weighted / volume)
}

/// Which extreme a swing point is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SwingKind {
    High,
    Low,
}

/// A fractal swing point.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Swing {
    pub index: usize,
    pub time: i64,
    pub price: f64,
    pub kind: SwingKind,
}

/// Fractal swing points: a bar higher (or lower) than `span` bars either side.
///
/// The last `span` bars can never qualify, because the bars that would confirm
/// them have not printed yet. A "swing high" called on the final bar is just
/// the current price with a name on it.
pub fn swings(candles: &[Candle], span: usize) -> Vec<Swing> {
    let mut found = Vec::new();
    for index in span..candles.len().saturating_sub(span) {
        let candle = candles[index];
        let window = &candles[index - span..=index + span];
        if window.iter().all(|other| other.high <= candle.high) {
            found.push(Swing {
                index,
                time: candle.time,
                price: candle.high,
                kind: SwingKind::High,
            });
        }
        if window.iter().all(|other| other.low >= candle.low) {
            found.push(Swing {
                index,
                time: candle.time,
                price: candle.low,
                kind: SwingKind::Low,
            });
        }
    }
    found
}

/// A price level price keeps returning to.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Level {
    pub price: f64,
    pub touches: u32,
}

/// Price levels touched more than once, strongest first. `span` defaults to 2
/// and `tolerance` to 0.015 in the TypeScript; see [`levels_default`].
///
/// Swing points are clustered by relative distance rather than absolute, so
/// this behaves the same on a token priced at 0.0007 and one priced at 40,000.
pub fn levels(candles: &[Candle], span: usize, tolerance: f64) -> Vec<Level> {
    let mut points = swings(candles, span);
    points.sort_by(|a, b| a.price.total_cmp(&b.price));
    let mut clusters: Vec<Level> = Vec::new();
    for point in points {
        match clusters.last_mut() {
            Some(last) if (point.price - last.price).abs() / last.price <= tolerance => {
                // Running mean, so a cluster's price is where the touches were
                // rather than wherever the first one happened to land.
                last.price = (last.price * f64::from(last.touches) + point.price)
                    / (f64::from(last.touches) + 1.0);
                last.touches += 1;
            }
            _ => clusters.push(Level {
                price: point.price,
                touches: 1,
            }),
        }
    }
    clusters.sort_by_key(|level| std::cmp::Reverse(level.touches));
    clusters
}

/// [`levels`] with the TypeScript's defaults: span 2, tolerance 1.5%.
pub fn levels_default(candles: &[Candle]) -> Vec<Level> {
    levels(candles, 2, 0.015)
}

/// The last defined value of a series, which is the only one a post quotes.
pub fn latest(series: &[Option<f64>]) -> Option<f64> {
    series.iter().rev().find_map(|value| *value)
}

#[cfg(test)]
#[path = "indicators_tests.rs"]
mod tests;
