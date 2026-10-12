//! Everything the agent is allowed to say about the tape, computed rather than
//! imagined.
//!
//! There are deliberately no oscillators. RSI and MACD were in the first
//! TypeScript version and came out again: they are mean-reversion tools, and a
//! memecoin does not revert. RSI read "overbought" at 74 through the move that
//! was the whole point of the day. What is left is what moves one of these:
//! levels defended more than once, what happens when one breaks, whether
//! volume showed up for it, and where price sits in its own range.
//!
//! Bearish signals are reported even though the account is unapologetically
//! long: a bullish poster that cannot see a downtrend eventually posts
//! "accumulation zone" into a chart that is bleeding.

use serde::{Deserialize, Serialize};

use crate::error::{MarketError, Result};
use crate::format::{js_number, js_round, round_to, to_precision};
use crate::ta::indicators::{
    Level, SwingKind, atr, bollinger, closes, ema, latest, levels_default, sma, swings, vwap,
};
use crate::types::{Candle, Timeframe};

/// How much a signal weighs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Strength {
    Weak,
    Moderate,
    Strong,
}

impl Strength {
    /// `weak`, `moderate`, `strong`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Weak => "weak",
            Self::Moderate => "moderate",
            Self::Strong => "strong",
        }
    }
}

/// One thing the chart says, and which way it reads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Signal {
    pub name: String,
    pub bullish: bool,
    pub strength: Strength,
    pub detail: String,
}

/// What happened at a level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LevelEventKind {
    Breakout,
    Breakdown,
    Reclaim,
}

impl LevelEventKind {
    /// `breakout`, `breakdown`, `reclaim`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Breakout => "breakout",
            Self::Breakdown => "breakdown",
            Self::Reclaim => "reclaim",
        }
    }
}

/// A level that has just been crossed, which is the only kind worth a post.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LevelEvent {
    pub kind: LevelEventKind,
    pub level: f64,
    pub touches: u32,
    /// Whether the bar that did it traded above its own average volume.
    pub volume_confirmed: bool,
    pub bars_ago: usize,
}

/// Which way the averages point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Trend {
    Up,
    Down,
    Sideways,
}

impl Trend {
    /// `up`, `down`, `sideways`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Down => "down",
            Self::Sideways => "sideways",
        }
    }
}

/// The two averages.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmaRead {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fast: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slow: Option<f64>,
    pub fast_period: usize,
    pub slow_period: usize,
}

/// ATR as a percentage of price, and how tight the range has become.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Volatility {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub atr_pct: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range_width_pct: Option<f64>,
}

/// Volume on the last bar and across the window.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct VolumeRead {
    pub last: f64,
    pub average: f64,
    pub ratio: f64,
    /// Recent half of the window against the half before it. Above 1 is waking up.
    pub trend: f64,
}

/// The nearest defended levels either side of price, and the window's range.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LevelsRead {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub support: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub support_touches: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resistance: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resistance_touches: Option<u32>,
    pub window_high: f64,
    pub window_low: f64,
}

/// The read of one chart.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TechnicalRead {
    pub timeframe: Timeframe,
    pub bars: usize,
    pub price: f64,
    /// Change over the whole window read, in percent.
    pub change_pct: f64,
    pub trend: Trend,
    pub structure: String,
    pub ema: EmaRead,
    pub volatility: Volatility,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vwap: Option<f64>,
    pub volume: VolumeRead,
    pub levels: LevelsRead,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<LevelEvent>,
    pub from_high_pct: f64,
    pub from_low_pct: f64,
    pub signals: Vec<Signal>,
    /// True, positive things about this chart, strongest first. Never empty.
    ///
    /// The post has to lead with one. Left to pick its own framing from a
    /// mixed read, a model opens with whatever is largest, and on a red day
    /// that is the loss.
    pub positives: Vec<String>,
    /// 0-100, where 50 is neutral: the balance of the signals, not a forecast.
    pub bullish_score: u32,
    /// Plain sentences a post can be built from, each true of the data above.
    pub notes: Vec<String>,
}

/// The EMA periods the read uses.
pub const FAST_PERIOD: usize = 20;
/// See [`FAST_PERIOD`].
pub const SLOW_PERIOD: usize = 50;

/// Reads a chart. Needs at least three candles.
pub fn analyse(candles: &[Candle], timeframe: Timeframe) -> Result<TechnicalRead> {
    if candles.len() < 3 {
        return Err(MarketError::data(
            "Not enough candles to analyse: need at least 3",
        ));
    }
    let values = closes(candles);
    let first = candles[0];
    let last = candles[candles.len() - 1];

    let fast = latest(&ema(&values, FAST_PERIOD));
    let slow = latest(&ema(&values, SLOW_PERIOD));
    let bands = bollinger(&values, 20, 2.0);
    let atr_value = latest(&atr(candles, 14));
    let volumes: Vec<f64> = candles.iter().map(|c| c.volume).collect();
    let volume_average = latest(&sma(&volumes, candles.len().min(20))).unwrap_or(0.0);

    let window_high = candles
        .iter()
        .map(|c| c.high)
        .fold(f64::NEG_INFINITY, f64::max);
    let window_low = candles.iter().map(|c| c.low).fold(f64::INFINITY, f64::min);
    let window_change = pct(first.open, last.close);

    let upper = latest(&bands.upper);
    let lower = latest(&bands.lower);
    let middle = latest(&bands.middle);

    let points = swings(candles, 2);
    let last_three = |kind: SwingKind| -> Vec<f64> {
        let prices: Vec<f64> = points
            .iter()
            .filter(|p| p.kind == kind)
            .map(|p| p.price)
            .collect();
        prices[prices.len().saturating_sub(3)..].to_vec()
    };
    let swing_highs = last_three(SwingKind::High);
    let swing_lows = last_three(SwingKind::Low);
    let higher_highs = rising(&swing_highs);
    let higher_lows = rising(&swing_lows);
    let lower_highs = falling(&swing_highs);
    let lower_lows = falling(&swing_lows);

    let found = levels_default(candles);
    // The nearest on each side; the first of equals, as a stable sort would give.
    let mut support: Option<Level> = None;
    let mut resistance: Option<Level> = None;
    for level in &found {
        if level.price < last.close && support.is_none_or(|best| level.price > best.price) {
            support = Some(*level);
        }
        if level.price > last.close && resistance.is_none_or(|best| level.price < best.price) {
            resistance = Some(*level);
        }
    }

    let event = find_event(candles, &found, volume_average);
    let volume_trend = trend_of(&volumes);

    let mut signals: Vec<Signal> = Vec::new();
    let mut add = |name: &str, bullish: bool, strength: Strength, detail: String| {
        signals.push(Signal {
            name: name.to_owned(),
            bullish,
            strength,
            detail,
        });
    };

    // The whole window, weighted heavily on purpose. Without it a token down
    // 50% over five days scores bullish on a two-hour bounce.
    add(
        "window trend",
        window_change >= 0.0,
        if window_change.abs() > 15.0 {
            Strength::Strong
        } else if window_change.abs() > 5.0 {
            Strength::Moderate
        } else {
            Strength::Weak
        },
        format!(
            "{} {}% across the {} bars read",
            if window_change >= 0.0 { "up" } else { "down" },
            js_number(round_to(window_change.abs(), 2)),
            candles.len()
        ),
    );

    // A level breaking is the most postable thing that happens on one of these
    // charts -- and downgraded without volume, because a break nobody traded
    // is a wick that gets given straight back.
    if let Some(event) = event {
        add(
            event.kind.as_str(),
            event.kind != LevelEventKind::Breakdown,
            if event.volume_confirmed {
                Strength::Strong
            } else {
                Strength::Moderate
            },
            format!(
                "{} ${} ({} touches) {}{}",
                match event.kind {
                    LevelEventKind::Breakdown => "lost",
                    LevelEventKind::Reclaim => "reclaimed",
                    LevelEventKind::Breakout => "broke above",
                },
                to_precision(event.level, 4),
                event.touches,
                bars_ago(event.bars_ago, "this bar"),
                if event.volume_confirmed {
                    ", on above-average volume"
                } else {
                    ", but volume did not confirm"
                }
            ),
        );
    }

    if let Some(support) = support {
        add(
            "support",
            true,
            if support.touches >= 3 {
                Strength::Strong
            } else {
                Strength::Moderate
            },
            format!(
                "${} has been defended {} times and is still below price",
                to_precision(support.price, 4),
                support.touches
            ),
        );
    }

    if let (Some(fast), Some(slow)) = (fast, slow) {
        let above = fast > slow;
        add(
            &format!("EMA{FAST_PERIOD}/EMA{SLOW_PERIOD}"),
            above,
            if pct(slow, fast).abs() > 2.0 {
                Strength::Strong
            } else {
                Strength::Moderate
            },
            if above {
                format!(
                    "the {FAST_PERIOD} EMA is {}% above the {SLOW_PERIOD}",
                    js_number(round_to(pct(slow, fast), 2))
                )
            } else {
                format!(
                    "the {FAST_PERIOD} EMA is {}% below the {SLOW_PERIOD}",
                    js_number(round_to(pct(slow, fast).abs(), 2))
                )
            },
        );
    }

    if higher_highs && higher_lows {
        add(
            "structure",
            true,
            Strength::Strong,
            "higher highs and higher lows".into(),
        );
    } else if lower_highs && lower_lows {
        add(
            "structure",
            false,
            Strength::Strong,
            "lower highs and lower lows".into(),
        );
    } else if higher_lows {
        add(
            "structure",
            true,
            Strength::Moderate,
            "higher lows into resistance".into(),
        );
    }

    if volume_average > 0.0 {
        let ratio = last.volume / volume_average;
        if ratio >= 1.5 {
            add(
                "volume spike",
                last.close >= last.open,
                if ratio >= 3.0 {
                    Strength::Strong
                } else {
                    Strength::Moderate
                },
                format!(
                    "last bar traded {}x the average volume",
                    js_number(round_to(ratio, 1))
                ),
            );
        }
    }
    if volume_trend >= 1.3 || volume_trend <= 0.7 {
        add(
            "volume trend",
            volume_trend >= 1.3,
            if volume_trend >= 2.0 || volume_trend <= 0.5 {
                Strength::Strong
            } else {
                Strength::Moderate
            },
            if volume_trend >= 1.3 {
                format!(
                    "volume in the recent half of the window is {}x the half before it",
                    js_number(round_to(volume_trend, 1))
                )
            } else {
                format!(
                    "volume has dried up to {}x the earlier half of the window",
                    js_number(round_to(volume_trend, 1))
                )
            },
        );
    }

    // A tight range is not bullish by itself. It is a setup: price is coiling,
    // and the next real move tends to come out of one.
    let range_width = match (upper, lower, middle) {
        (Some(upper), Some(lower), Some(middle)) if middle != 0.0 => {
            Some(((upper - lower) / middle) * 100.0)
        }
        _ => None,
    };
    if let Some(width) = range_width.filter(|width| *width < 6.0) {
        add(
            "range tightening",
            true,
            Strength::Moderate,
            format!(
                "the range has narrowed to {}% wide, which is where moves start",
                js_number(round_to(width, 2))
            ),
        );
    }

    let drawdown = pct(window_high, last.close);
    if drawdown > -3.0 {
        add(
            "range position",
            true,
            if drawdown > -1.0 {
                Strength::Strong
            } else {
                Strength::Moderate
            },
            format!(
                "trading within {}% of the window high",
                js_number(round_to(drawdown.abs(), 2))
            ),
        );
    }

    let bullish_weight: u32 = signals
        .iter()
        .filter(|s| s.bullish)
        .map(|s| weight(s.strength))
        .sum();
    let total: u32 = signals.iter().map(|s| weight(s.strength)).sum();
    let bullish_score = if total == 0 {
        50
    } else {
        js_round((f64::from(bullish_weight) / f64::from(total)) * 100.0) as u32
    };

    let trend = match (fast, slow) {
        (Some(fast), Some(slow)) if pct(slow, fast).abs() < 0.5 => Trend::Sideways,
        (Some(fast), Some(slow)) if fast > slow => Trend::Up,
        (Some(_), Some(_)) => Trend::Down,
        _ => Trend::Sideways,
    };

    let mut read = TechnicalRead {
        timeframe,
        bars: candles.len(),
        price: last.close,
        change_pct: round_to(window_change, 2),
        trend,
        structure: if higher_highs && higher_lows {
            "higher highs, higher lows"
        } else if lower_highs && lower_lows {
            "lower highs, lower lows"
        } else if higher_lows {
            "higher lows"
        } else {
            "no clean structure"
        }
        .to_owned(),
        ema: EmaRead {
            fast,
            slow,
            fast_period: FAST_PERIOD,
            slow_period: SLOW_PERIOD,
        },
        volatility: Volatility {
            atr_pct: atr_value.map(|value| round_to((value / last.close) * 100.0, 2)),
            range_width_pct: range_width.map(|width| round_to(width, 2)),
        },
        vwap: vwap(candles),
        volume: VolumeRead {
            last: last.volume,
            average: volume_average,
            ratio: if volume_average > 0.0 {
                round_to(last.volume / volume_average, 2)
            } else {
                0.0
            },
            trend: round_to(volume_trend, 2),
        },
        levels: LevelsRead {
            support: support.map(|level| level.price),
            support_touches: support.map(|level| level.touches),
            resistance: resistance.map(|level| level.price),
            resistance_touches: resistance.map(|level| level.touches),
            window_high,
            window_low,
        },
        event,
        from_high_pct: round_to(drawdown, 2),
        from_low_pct: round_to(pct(window_low, last.close), 2),
        signals,
        positives: Vec::new(),
        bullish_score,
        notes: Vec::new(),
    };
    read.positives = find_positives(&read, candles);
    read.notes = describe(&read);
    Ok(read)
}

fn weight(strength: Strength) -> u32 {
    match strength {
        Strength::Weak => 1,
        Strength::Moderate => 2,
        Strength::Strong => 3,
    }
}

fn pct(from: f64, to: f64) -> f64 {
    if from == 0.0 {
        0.0
    } else {
        ((to - from) / from) * 100.0
    }
}

/// `this bar` / `3 bars ago`, as the TypeScript spelled it.
fn bars_ago(bars: usize, zero: &str) -> String {
    if bars == 0 {
        zero.to_owned()
    } else {
        format!("{bars} bars ago")
    }
}

/// Everything true and good that can be said about this chart, best first.
///
/// The order is the order a post should reach for them in: a defended level
/// beats a percentage, a percentage beats a ratio, and something happening now
/// beats something that happened last week. The last entry is a floor: price
/// is always some distance above its own low.
fn find_positives(read: &TechnicalRead, candles: &[Candle]) -> Vec<String> {
    let mut positives = Vec::new();
    let last = candles[candles.len() - 1];

    if let Some(event) = read.event.filter(|e| e.kind != LevelEventKind::Breakdown) {
        positives.push(format!(
            "{} ${}{}",
            if event.kind == LevelEventKind::Reclaim {
                "reclaimed"
            } else {
                "broke above"
            },
            to_precision(event.level, 4),
            if event.volume_confirmed {
                " with volume behind it"
            } else {
                ""
            }
        ));
    }
    if let Some(support) = read.levels.support {
        positives.push(format!(
            "${} has been defended {} times and price is still above it",
            to_precision(support, 4),
            read.levels.support_touches.unwrap_or(2)
        ));
    }
    // A level that broke is not the end of the story: where price stopped
    // falling, if it has held there for a few bars, is a new base.
    if let Some((price, bars)) = find_base(read, candles) {
        positives.push(format!(
            "a new base is forming at ${}, held for {bars} bars since the level broke",
            to_precision(price, 4)
        ));
    }
    if read.structure == "higher highs, higher lows" || read.structure == "higher lows" {
        positives.push(format!("the lows are stepping up ({})", read.structure));
    }
    if read.change_pct > 0.0 {
        positives.push(format!(
            "up {}% across the window read",
            js_number(read.change_pct)
        ));
    }
    if read.volume.trend >= 1.2 {
        positives.push(format!(
            "volume is {}x what it was earlier in the window",
            js_number(read.volume.trend)
        ));
    }
    if read.volume.ratio >= 1.5 && last.close >= last.open {
        positives.push(format!(
            "the last bar traded {}x average volume and closed green",
            js_number(read.volume.ratio)
        ));
    }
    if read.ema.fast.is_some_and(|fast| last.close > fast) {
        positives.push(format!(
            "price is holding above the {} EMA",
            read.ema.fast_period
        ));
    }
    if let Some(width) = read.volatility.range_width_pct.filter(|width| *width < 6.0) {
        positives.push(format!(
            "the range has coiled to {}% wide",
            js_number(width)
        ));
    }
    let recent = &candles[candles.len().saturating_sub(24)..];
    let green = recent.iter().filter(|c| c.close >= c.open).count();
    if green as f64 > recent.len() as f64 / 2.0 {
        positives.push(format!(
            "{green} of the last {} bars closed green",
            recent.len()
        ));
    }
    if read.from_high_pct > -5.0 {
        positives.push(format!(
            "price is within {}% of the window high",
            js_number(read.from_high_pct.abs())
        ));
    }
    positives.push(format!(
        "price is {}% up off the low of the window",
        js_number(read.from_low_pct)
    ));
    positives
}

/// The most recent level crossing, if there was one worth mentioning.
///
/// Only levels with more than one touch count: a line price has been to once
/// is a coordinate, not a level.
fn find_event(candles: &[Candle], found: &[Level], volume_average: f64) -> Option<LevelEvent> {
    let real: Vec<&Level> = found.iter().filter(|level| level.touches >= 2).collect();
    if real.is_empty() {
        return None;
    }
    // Only the recent past: a break twelve bars ago is history, not news.
    let lookback = 12.min(candles.len() - 1);
    for back in 0..lookback {
        let index = candles.len() - 1 - back;
        let (candle, previous) = (candles[index], candles[index - 1]);
        for level in &real {
            let crossed_up = previous.close <= level.price && candle.close > level.price;
            let crossed_down = previous.close >= level.price && candle.close < level.price;
            if !crossed_up && !crossed_down {
                continue;
            }
            // A reclaim rather than a breakout when price had been under this
            // level a while and came back over: the two read differently to
            // somebody who held through the part where it was lost.
            let lost_earlier = candles[index.saturating_sub(24)..index]
                .iter()
                .any(|bar| bar.close < level.price);
            return Some(LevelEvent {
                kind: if crossed_down {
                    LevelEventKind::Breakdown
                } else if lost_earlier {
                    LevelEventKind::Reclaim
                } else {
                    LevelEventKind::Breakout
                },
                level: level.price,
                touches: level.touches,
                volume_confirmed: volume_average > 0.0 && candle.volume > volume_average,
                bars_ago: back,
            });
        }
    }
    None
}

/// Where price stopped falling after a level broke, if it has stayed there:
/// the floor and how many bars came back down to it and held.
///
/// Three bars is the floor: one bar is a wick and two is a pause. Below that
/// this would be calling a base on the way down.
fn find_base(read: &TechnicalRead, candles: &[Candle]) -> Option<(f64, usize)> {
    let event = read.event?;
    if event.kind != LevelEventKind::Breakdown || event.bars_ago < 3 {
        return None;
    }
    let since = &candles[candles.len() - 1 - event.bars_ago..];
    if since.len() < 3 {
        return None;
    }
    let floor = since.iter().map(|c| c.low).fold(f64::INFINITY, f64::min);
    let last = candles[candles.len() - 1];
    if last.close <= floor {
        return None;
    }
    // If the last bar set the low, price is still falling and there is no base.
    if since[since.len() - 1].low == floor {
        return None;
    }
    // Bars that came back down to the floor and held, not bars merely above it.
    let held = since.iter().filter(|c| c.low <= floor * 1.005).count();
    (held >= 3).then_some((floor, held))
}

/// Volume in the recent half of the window against the half before it.
fn trend_of(volumes: &[f64]) -> f64 {
    if volumes.len() < 4 {
        return 1.0;
    }
    let half = volumes.len() / 2;
    let mean = |values: &[f64]| {
        if values.is_empty() {
            0.0
        } else {
            values.iter().fold(0.0, |total, value| total + value) / values.len() as f64
        }
    };
    let earlier = mean(&volumes[..half]);
    let recent = mean(&volumes[half..]);
    if earlier == 0.0 {
        if recent > 0.0 { 2.0 } else { 1.0 }
    } else {
        recent / earlier
    }
}

/// The read as sentences, each quotable in a post without further checking.
fn describe(read: &TechnicalRead) -> Vec<String> {
    let mut notes = vec![
        format!(
            "Over the last {} {} bars price is {} {}%, now at ${}.",
            read.bars,
            read.timeframe,
            if read.change_pct >= 0.0 { "up" } else { "down" },
            js_number(read.change_pct.abs()),
            to_precision(read.price, 4)
        ),
        format!(
            "Trend reads {}; structure is {}.",
            read.trend.as_str(),
            read.structure
        ),
    ];
    if let Some(event) = read.event {
        notes.push(format!(
            "{} ${} {}{}",
            match event.kind {
                LevelEventKind::Breakdown => "Lost",
                LevelEventKind::Reclaim => "Reclaimed",
                LevelEventKind::Breakout => "Broke above",
            },
            to_precision(event.level, 4),
            bars_ago(event.bars_ago, "on the current bar"),
            if event.volume_confirmed {
                ", with volume behind it."
            } else {
                ", but volume did not confirm it."
            }
        ));
    }
    let touches = |count: Option<u32>| count.map(|n| format!(" ({n} touches)")).unwrap_or_default();
    if let Some(support) = read.levels.support {
        notes.push(format!(
            "Nearest support that has actually been defended: ${}{}.",
            to_precision(support, 4),
            touches(read.levels.support_touches)
        ));
    }
    if let Some(resistance) = read.levels.resistance {
        notes.push(format!(
            "Nearest overhead level: ${}{}.",
            to_precision(resistance, 4),
            touches(read.levels.resistance_touches)
        ));
    }
    notes.push(format!(
        "Window high ${}, low ${}; price sits {}% off the high and {}% off the low.",
        to_precision(read.levels.window_high, 4),
        to_precision(read.levels.window_low, 4),
        js_number(read.from_high_pct.abs()),
        js_number(read.from_low_pct)
    ));
    notes.push(format!(
        "Volume: last bar {}x its average, and the recent half of the window is {}x the half before it.",
        js_number(read.volume.ratio),
        js_number(read.volume.trend)
    ));
    notes.push(format!(
        "TRUE POSITIVES YOU CAN LEAD WITH, best first: {}.",
        read.positives.join("; ")
    ));
    notes.push(format!(
        "Signal balance: {}/100 bullish ({} of {} signals reading up).",
        read.bullish_score,
        read.signals.iter().filter(|s| s.bullish).count(),
        read.signals.len()
    ));
    notes
}

fn rising(values: &[f64]) -> bool {
    values.len() >= 2 && values.windows(2).all(|pair| pair[1] > pair[0])
}

fn falling(values: &[f64]) -> bool {
    values.len() >= 2 && values.windows(2).all(|pair| pair[1] < pair[0])
}

#[cfg(test)]
#[path = "analysis_tests.rs"]
mod tests;
