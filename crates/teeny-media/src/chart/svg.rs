//! A candlestick chart as SVG text.

use teeny_core::config::ChartTimeframe;

use super::theme::THEME;
use crate::candle::Candle;
use crate::error::{Error, Result};
use crate::format::{
    axis_time, escape_xml, js_number, price, signed_percent, to_fixed, to_precision_number,
};

/// An indicator drawn over the candles.
#[derive(Debug, Clone, PartialEq)]
pub struct Overlay {
    pub label: String,
    pub color: String,
    /// Aligned to the candles; `None` where the indicator has no value yet.
    pub values: Vec<Option<f64>>,
}

/// Support, resistance and similar, drawn as a dashed rule.
#[derive(Debug, Clone, PartialEq)]
pub struct HorizontalLine {
    pub label: String,
    pub value: f64,
    pub color: Option<String>,
}

/// One entry of the strip under the title: market cap, liquidity, volume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stat {
    pub label: String,
    pub value: String,
}

/// Everything the chart draws.
#[derive(Debug, Clone, PartialEq)]
pub struct ChartInput {
    pub candles: Vec<Candle>,
    pub symbol: String,
    pub name: Option<String>,
    pub timeframe: ChartTimeframe,
    /// The live price, which may be newer than the last candle's close.
    pub price_usd: Option<f64>,
    pub change_pct: Option<f64>,
    pub overlays: Vec<Overlay>,
    pub lines: Vec<HorizontalLine>,
    pub stats: Vec<Stat>,
    pub watermark: Option<String>,
    /// Default 1200.
    pub width: Option<u32>,
    /// Default 675.
    pub height: Option<u32>,
}

impl ChartInput {
    /// Candles, a symbol and a timeframe; every other field empty.
    pub fn new(candles: Vec<Candle>, symbol: impl Into<String>, timeframe: ChartTimeframe) -> Self {
        Self {
            candles,
            symbol: symbol.into(),
            name: None,
            timeframe,
            price_usd: None,
            change_pct: None,
            overlays: Vec::new(),
            lines: Vec::new(),
            stats: Vec::new(),
            watermark: None,
            width: None,
            height: None,
        }
    }
}

const PADDING_TOP: f64 = 92.0;
const PADDING_RIGHT: f64 = 96.0;
const PADDING_BOTTOM: f64 = 36.0;
const PADDING_LEFT: f64 = 16.0;
const VOLUME_FRACTION: f64 = 0.18;
const GAP: f64 = 16.0;

/// A candlestick chart as SVG text.
///
/// SVG rather than a drawing library because it is a string: the tests assert
/// on it, a diff shows what changed, and the only native dependency in the
/// whole path is the rasteriser that turns it into the PNG X actually accepts.
/// Every number is spelled as the TypeScript renderer spelled it, so the
/// output is the legacy SVG byte for byte.
pub fn render_chart_svg(input: &ChartInput) -> Result<String> {
    let candles = &input.candles;
    let Some(last) = candles.last() else {
        return Err(Error::failed("Cannot draw a chart with no candles"));
    };
    let n = |value: f64| js_number(value);

    let width = f64::from(input.width.unwrap_or(1_200));
    let height = f64::from(input.height.unwrap_or(675));
    let plot_left = PADDING_LEFT;
    let plot_right = width - PADDING_RIGHT;
    let plot_width = plot_right - plot_left;
    let volume_height = (height - PADDING_TOP - PADDING_BOTTOM) * VOLUME_FRACTION;
    let price_top = PADDING_TOP;
    let price_bottom = height - PADDING_BOTTOM - volume_height - GAP;
    let price_height = price_bottom - price_top;
    let volume_bottom = height - PADDING_BOTTOM;

    // The vertical scale covers the overlays too. An EMA that leaves the panel
    // is worse than no EMA: it reads as the price having gone somewhere it did
    // not.
    let extras = input
        .overlays
        .iter()
        .flat_map(|overlay| overlay.values.iter().flatten().copied())
        .chain(input.lines.iter().map(|line| line.value));
    let highest = js_max(candles.iter().map(|candle| candle.h).chain(extras.clone()));
    let lowest = js_min(candles.iter().map(|candle| candle.l).chain(extras));
    let span = or(or(highest - lowest, highest), 1.0);
    let top = highest + span * 0.08;
    let bottom = (lowest - span * 0.08).max(0.0);

    let y = |value: f64| price_bottom - ((value - bottom) / or(top - bottom, 1.0)) * price_height;
    let count = candles.len() as f64;
    let slot = plot_width / count;
    let body_width = (slot * 0.68).min(22.0).max(1.0);
    let x = |index: usize| plot_left + slot * (index as f64 + 0.5);

    // The volume panel is scaled to the 95th percentile, not to the maximum.
    // One launch bar or one whale print is routinely fifty times the median,
    // and scaling to it flattens every other bar into the axis. Bars over the
    // cap are clipped to the top of the panel, which reads correctly: that
    // bar is off the scale.
    let volumes: Vec<f64> = candles.iter().map(|candle| candle.v).collect();
    let volume_cap = or(percentile(&volumes, 0.95) * 1.25, 1.0);
    let volume_y = |value: f64| volume_bottom - (value.min(volume_cap) / volume_cap) * volume_height;

    let shown_price = input.price_usd.unwrap_or(last.c);
    let change = input.change_pct;
    let change_color = match change {
        None => THEME.muted,
        Some(change) if change >= 0.0 => THEME.up,
        Some(_) => THEME.down,
    };

    let mut parts: Vec<String> = vec![
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}" font-family="{font}">"#,
            w = n(width),
            h = n(height),
            font = THEME.font,
        ),
        format!(r#"<rect width="{}" height="{}" fill="{}"/>"#, n(width), n(height), THEME.background),
    ];

    // Watermark first, so every later element sits on top of it.
    if let Some(watermark) = input.watermark.as_deref().filter(|mark| !mark.is_empty()) {
        parts.push(format!(
            r#"<text x="{}" y="{}" fill="{}" font-size="72" font-weight="700" text-anchor="middle">{}</text>"#,
            n(plot_left + plot_width / 2.0),
            n(price_top + price_height / 2.0 + 24.0),
            THEME.watermark,
            escape_xml(watermark),
        ));
    }

    // Horizontal grid and the price axis on the right, where a chart puts it.
    for tick in price_ticks(bottom, top, 6) {
        let line = y(tick);
        parts.push(format!(
            r#"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{}" stroke-width="1"/>"#,
            n(plot_left),
            n(line),
            n(plot_right),
            n(line),
            THEME.grid,
        ));
        parts.push(format!(
            r#"<text x="{}" y="{}" fill="{}" font-size="13">{}</text>"#,
            n(plot_right + 8.0),
            n(line + 4.0),
            THEME.muted,
            escape_xml(&price(tick)),
        ));
    }
    parts.push(format!(
        r#"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{}" stroke-width="1"/>"#,
        n(plot_right),
        n(price_top),
        n(plot_right),
        n(volume_bottom),
        THEME.axis,
    ));
    parts.push(format!(
        r#"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{}" stroke-width="1"/>"#,
        n(plot_left),
        n(volume_bottom),
        n(plot_right),
        n(volume_bottom),
        THEME.axis,
    ));

    // Time axis: about six labels, whatever the bar count.
    let step = (candles.len() / 6).max(1);
    for (index, candle) in candles.iter().enumerate().step_by(step) {
        let at = x(index);
        parts.push(format!(
            r#"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{}" stroke-width="1"/>"#,
            n(at),
            n(price_top),
            n(at),
            n(volume_bottom),
            THEME.grid,
        ));
        parts.push(format!(
            r#"<text x="{}" y="{}" fill="{}" font-size="12" text-anchor="middle">{}</text>"#,
            n(at),
            n(volume_bottom + 20.0),
            THEME.muted,
            escape_xml(&axis_time(candle.t, input.timeframe.as_str())),
        ));
    }

    for line in &input.lines {
        let at = y(line.value);
        if at < price_top || at > price_bottom {
            continue;
        }
        parts.push(format!(
            r#"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{}" stroke-width="1" stroke-dasharray="6 5"/>"#,
            n(plot_left),
            n(at),
            n(plot_right),
            n(at),
            line.color.as_deref().unwrap_or(THEME.level),
        ));
        parts.push(format!(
            r#"<text x="{}" y="{}" fill="{}" font-size="12">{}</text>"#,
            n(plot_left + 8.0),
            n(at - 6.0),
            line.color.as_deref().unwrap_or(THEME.muted),
            escape_xml(&line.label),
        ));
    }

    // Volume, under the price panel and behind nothing.
    for (index, candle) in candles.iter().enumerate() {
        let bar_height = (volume_bottom - volume_y(candle.v)).max(0.0);
        parts.push(format!(
            r#"<rect x="{}" y="{}" width="{}" height="{}" fill="{}"/>"#,
            n(x(index) - body_width / 2.0),
            n(volume_y(candle.v)),
            n(body_width),
            n(bar_height),
            if candle.c >= candle.o { THEME.up_volume } else { THEME.down_volume },
        ));
    }

    // Candles. The wick is drawn first so the body covers its middle.
    for (index, candle) in candles.iter().enumerate() {
        let colour = if candle.c >= candle.o { THEME.up } else { THEME.down };
        let at = x(index);
        let body_top = y(candle.o.max(candle.c));
        // A doji has a zero-height body, which renders as nothing at all; one
        // pixel is what makes an unchanged bar still visible.
        let body_height = (y(candle.o.min(candle.c)) - body_top).max(1.0);
        parts.push(format!(
            r#"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{}" stroke-width="1.5"/>"#,
            n(at),
            n(y(candle.h)),
            n(at),
            n(y(candle.l)),
            colour,
        ));
        parts.push(format!(
            r#"<rect x="{}" y="{}" width="{}" height="{}" fill="{}"/>"#,
            n(at - body_width / 2.0),
            n(body_top),
            n(body_width),
            n(body_height),
            colour,
        ));
    }

    for overlay in &input.overlays {
        let path = polyline(&overlay.values, &x, &y);
        if path.is_empty() {
            continue;
        }
        parts.push(format!(
            r#"<path d="{}" fill="none" stroke="{}" stroke-width="1.8" stroke-linejoin="round" stroke-linecap="round" opacity="0.9"/>"#,
            path, overlay.color,
        ));
    }

    // The last-price tag on the axis, the one thing everybody actually reads.
    let tag_y = y(last.c);
    let tag_colour = if last.c >= last.o { THEME.up } else { THEME.down };
    parts.push(format!(
        r#"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{}" stroke-width="1" stroke-dasharray="3 3" opacity="0.7"/>"#,
        n(plot_left),
        n(tag_y),
        n(plot_right),
        n(tag_y),
        tag_colour,
    ));
    parts.push(format!(
        r#"<rect x="{}" y="{}" width="{}" height="22" rx="3" fill="{}"/>"#,
        n(plot_right + 1.0),
        n(tag_y - 11.0),
        n(PADDING_RIGHT - 6.0),
        tag_colour,
    ));
    parts.push(format!(
        r##"<text x="{}" y="{}" fill="#0b0e11" font-size="13" font-weight="700">{}</text>"##,
        n(plot_right + 8.0),
        n(tag_y + 4.0),
        escape_xml(&price(last.c)),
    ));

    // Header.
    parts.push(format!(
        r#"<text x="{}" y="34" fill="{}" font-size="26" font-weight="700">{}</text>"#,
        n(plot_left),
        THEME.text,
        escape_xml(&input.symbol),
    ));
    parts.push(format!(
        r#"<text x="{}" y="34" fill="{}" font-size="15">{}</text>"#,
        n(plot_left + js_len(&input.symbol) * 16.0 + 12.0),
        THEME.muted,
        escape_xml(&format!(
            "{} · {} · {} bars",
            input.name.as_deref().unwrap_or(""),
            input.timeframe.as_str(),
            candles.len()
        )),
    ));
    parts.push(format!(
        r#"<text x="{}" y="64" fill="{}" font-size="22" font-weight="600">{}</text>"#,
        n(plot_left),
        THEME.text,
        escape_xml(&format!("${}", price(shown_price))),
    ));
    if let Some(change) = change {
        parts.push(format!(
            r#"<text x="{}" y="64" fill="{}" font-size="18" font-weight="600">{}</text>"#,
            n(plot_left + 130.0),
            change_color,
            escape_xml(&signed_percent(change)),
        ));
    }

    let mut offset = plot_left + 230.0;
    for stat in &input.stats {
        let text = format!("{} {}", stat.label, stat.value);
        parts.push(format!(
            r#"<text x="{}" y="64" fill="{}" font-size="15">{}</text>"#,
            n(offset),
            THEME.muted,
            escape_xml(&text),
        ));
        offset += js_len(&text) * 8.2 + 26.0;
    }

    // The overlay legend goes top-right, out of the way of the price.
    let mut legend_x = plot_right;
    for overlay in input.overlays.iter().rev() {
        legend_x -= js_len(&overlay.label) * 8.0 + 22.0;
        parts.push(format!(
            r#"<text x="{}" y="{}" fill="{}" font-size="13" font-weight="600">{}</text>"#,
            n(legend_x),
            n(price_top - 10.0),
            overlay.color,
            escape_xml(&overlay.label),
        ));
    }

    parts.push("</svg>".to_owned());
    Ok(parts.join("\n"))
}

/// `None` breaks the line rather than interpolating across the gap.
fn polyline(values: &[Option<f64>], x: &impl Fn(usize) -> f64, y: &impl Fn(f64) -> f64) -> String {
    let mut segments: Vec<String> = Vec::new();
    let mut drawing = false;
    for (index, value) in values.iter().enumerate() {
        let Some(value) = value else {
            drawing = false;
            continue;
        };
        segments.push(format!(
            "{}{} {}",
            if drawing { 'L' } else { 'M' },
            to_fixed(x(index), 2),
            to_fixed(y(*value), 2)
        ));
        drawing = true;
    }
    segments.join(" ")
}

/// The value below which `fraction` of the sorted inputs fall.
pub fn percentile(values: &[f64], fraction: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let index = ((sorted.len() as f64 * fraction).floor() as usize).min(sorted.len() - 1);
    sorted[index]
}

/// Round-ish gridline values inside the range, in the token's own order of
/// magnitude.
pub fn price_ticks(low: f64, high: f64, count: usize) -> Vec<f64> {
    if !(high > low) {
        return vec![low];
    }
    let raw = (high - low) / count as f64;
    // Parsed rather than computed: `1e-5` read from text is the correctly
    // rounded power, which is what `10 ** n` gives in JavaScript.
    let magnitude: f64 = format!("1e{}", raw.log10().floor()).parse().unwrap_or(1.0);
    let normalised = raw / magnitude;
    let step = if normalised >= 5.0 {
        5.0
    } else if normalised >= 2.0 {
        2.0
    } else {
        1.0
    } * magnitude;
    let mut ticks = Vec::new();
    let mut value = (low / step).ceil() * step;
    while value <= high {
        ticks.push(to_precision_number(value, 12));
        value += step;
    }
    ticks
}

/// `a || b` on numbers: zero and NaN are falsy.
fn or(value: f64, fallback: f64) -> f64 {
    if value == 0.0 || value.is_nan() { fallback } else { value }
}

/// `Math.max(...)`, NaN poisoning included.
fn js_max(values: impl Iterator<Item = f64>) -> f64 {
    values.fold(f64::NEG_INFINITY, |max, value| if value.is_nan() || max.is_nan() { f64::NAN } else { max.max(value) })
}

/// `Math.min(...)`, NaN poisoning included.
fn js_min(values: impl Iterator<Item = f64>) -> f64 {
    values.fold(f64::INFINITY, |min, value| if value.is_nan() || min.is_nan() { f64::NAN } else { min.min(value) })
}

/// `string.length`: UTF-16 code units, which is what the layout counted.
pub(crate) fn js_len(text: &str) -> f64 {
    text.encode_utf16().count() as f64
}

#[cfg(test)]
#[path = "svg_tests.rs"]
pub(crate) mod tests;
