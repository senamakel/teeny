//! The long horizon, which is the part a holder actually lives in.
//!
//! The hourly read answers "what happened today". This answers "are we further
//! along than we were": changes over weeks, how many days closed green,
//! whether the lows are stepping up. No indicator at all -- on a daily
//! memecoin series there is not enough history for one to mean anything, and
//! the honest long-term signals are arithmetic.

use serde::{Deserialize, Serialize};

use crate::error::{MarketError, Result};
use crate::format::{js_number, round_to, to_precision};
use crate::ta::indicators::{SwingKind, swings};
use crate::types::Candle;

/// Change over one horizon.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HorizonChange {
    /// How many days back, e.g. 7.
    pub days: usize,
    pub change_pct: f64,
}

/// The long-horizon read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub days: usize,
    pub price: f64,
    pub changes: Vec<HorizonChange>,
    /// Highest and lowest close in the window, and how far price is from each.
    pub high: f64,
    pub low: f64,
    pub from_high_pct: f64,
    pub from_low_pct: f64,
    /// Days closed green out of the days read.
    pub green_days: usize,
    /// Whether the daily lows are stepping up.
    pub higher_lows: bool,
    /// True, positive things about the long horizon, strongest first. Never empty.
    pub positives: Vec<String>,
    pub notes: Vec<String>,
}

/// Reads progress off daily candles. Needs at least two.
pub fn read_progress(daily: &[Candle]) -> Result<Progress> {
    let (Some(first), Some(last)) = (daily.first(), daily.last()) else {
        return Err(not_enough());
    };
    if daily.len() < 2 {
        return Err(not_enough());
    }

    let high = daily
        .iter()
        .map(|c| c.close)
        .fold(f64::NEG_INFINITY, f64::max);
    let low = daily.iter().map(|c| c.close).fold(f64::INFINITY, f64::min);

    let mut changes: Vec<HorizonChange> = [7, 30, 90]
        .into_iter()
        .filter(|&days| daily.len() > days)
        .map(|days| HorizonChange {
            days,
            change_pct: round_to(pct(daily[daily.len() - 1 - days].close, last.close), 2),
        })
        .collect();
    // Whatever history there is, if it is shorter than the shortest horizon.
    // A token that launched three weeks ago still has a story.
    if changes.is_empty() {
        changes.push(HorizonChange {
            days: daily.len() - 1,
            change_pct: round_to(pct(first.close, last.close), 2),
        });
    }

    let lows: Vec<f64> = swings(daily, 2)
        .into_iter()
        .filter(|point| point.kind == SwingKind::Low)
        .map(|point| point.price)
        .collect();
    let lows = &lows[lows.len().saturating_sub(3)..];

    let days = daily.len();
    let from_high_pct = round_to(pct(high, last.close), 2);
    let from_low_pct = round_to(pct(low, last.close), 2);
    let green_days = daily.iter().filter(|c| c.close >= c.open).count();
    let higher_lows = lows.len() >= 2 && lows.windows(2).all(|pair| pair[1] > pair[0]);

    // The long horizon is where the positive usually lives on a red day: an
    // hour can be ugly while the month is not, and both are true at once.
    let mut positives: Vec<String> = changes
        .iter()
        .filter(|change| change.change_pct > 0.0)
        .map(|change| {
            format!(
                "up {}% over {} days",
                js_number(change.change_pct),
                change.days
            )
        })
        .collect();
    if higher_lows {
        positives.push("the daily lows are still stepping up".into());
    }
    if green_days as f64 > days as f64 / 2.0 {
        positives.push(format!("{green_days} of the last {days} days closed green"));
    }
    positives.push(format!(
        "price is {}% above the low of this stretch",
        js_number(from_low_pct)
    ));

    let horizons: Vec<String> = changes
        .iter()
        .map(|change| {
            format!(
                "{}d {}{}%",
                change.days,
                if change.change_pct >= 0.0 { "+" } else { "" },
                js_number(change.change_pct)
            )
        })
        .collect();
    let notes = vec![
        format!("Over {days} days: {}.", horizons.join(", ")),
        format!("{green_days} of {days} days closed green."),
        format!(
            "Daily high ${}, low ${}; price is {}% off the high and {}% off the low.",
            to_precision(high, 4),
            to_precision(low, 4),
            js_number(from_high_pct.abs()),
            js_number(from_low_pct)
        ),
        if higher_lows {
            "On the daily, the lows are stepping up."
        } else {
            "On the daily, the lows are not yet stepping up."
        }
        .to_owned(),
        format!(
            "TRUE POSITIVES YOU CAN LEAD WITH, best first: {}.",
            positives.join("; ")
        ),
    ];

    Ok(Progress {
        days,
        price: last.close,
        changes,
        high,
        low,
        from_high_pct,
        from_low_pct,
        green_days,
        higher_lows,
        positives,
        notes,
    })
}

fn not_enough() -> MarketError {
    MarketError::data("Not enough daily candles to read progress: need at least 2")
}

fn pct(from: f64, to: f64) -> f64 {
    if from == 0.0 {
        0.0
    } else {
        ((to - from) / from) * 100.0
    }
}

#[cfg(test)]
#[path = "progress_tests.rs"]
mod tests;
