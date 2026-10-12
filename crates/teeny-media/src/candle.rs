//! The chart's input bar.

/// One OHLCV bar, the shape the chart draws.
///
/// Local on purpose: the market crate has its own candle and maps onto this
/// one, so the renderer does not depend on where the data came from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Candle {
    /// Bar open time, unix seconds.
    pub t: i64,
    pub o: f64,
    pub h: f64,
    pub l: f64,
    pub c: f64,
    pub v: f64,
}
