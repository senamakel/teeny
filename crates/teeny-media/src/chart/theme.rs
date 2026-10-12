//! The palette, kept in one place so a chart posted today looks like the one
//! posted last week. It is TradingView's dark theme on purpose: it is what the
//! timeline expects a chart to look like, and a chart that looks unfamiliar
//! reads as fake before anybody has looked at the candles.

/// The chart palette and font stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub background: &'static str,
    pub panel: &'static str,
    pub grid: &'static str,
    pub axis: &'static str,
    pub text: &'static str,
    pub muted: &'static str,
    pub up: &'static str,
    pub down: &'static str,
    pub up_volume: &'static str,
    pub down_volume: &'static str,
    pub fast: &'static str,
    pub slow: &'static str,
    pub level: &'static str,
    pub watermark: &'static str,
    pub font: &'static str,
}

/// The one theme every chart is drawn in.
pub const THEME: Theme = Theme {
    background: "#131722",
    panel: "#131722",
    grid: "#1e222d",
    axis: "#2a2e39",
    text: "#d1d4dc",
    muted: "#787b86",
    up: "#26a69a",
    down: "#ef5350",
    up_volume: "#26a69a55",
    down_volume: "#ef535055",
    fast: "#f0b90b",
    slow: "#2962ff",
    level: "#787b8688",
    watermark: "#ffffff10",
    font: "'Noto Sans', 'DejaVu Sans', 'Helvetica Neue', Helvetica, Arial, sans-serif",
};
