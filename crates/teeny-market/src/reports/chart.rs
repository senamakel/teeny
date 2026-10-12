//! `technical_analysis`, `chart_timeframes` and `token_progress`.

use serde::{Deserialize, Serialize};

use super::Report;
use crate::format::{js_number, to_precision};
use crate::ta::analysis::TechnicalRead;
use crate::ta::progress::Progress;
use crate::types::Timeframe;

/// `technical_analysis`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TechnicalAnalysisReport {
    pub timeframe: Timeframe,
    /// Bars that came back.
    pub bars: usize,
    /// `None` under 20 bars: not enough to analyse.
    pub read: Option<TechnicalRead>,
}

impl Report for TechnicalAnalysisReport {
    fn text(&self) -> String {
        let Some(read) = &self.read else {
            return format!("Only {} bars available; not enough to analyse.", self.bars);
        };
        let mut lines = read.notes.clone();
        lines.push(String::new());
        lines.push("signals:".into());
        lines.extend(read.signals.iter().map(|signal| {
            format!(
                "- {}: {} [{}, {}]",
                signal.name,
                signal.detail,
                if signal.bullish { "bullish" } else { "bearish" },
                signal.strength.as_str()
            )
        }));
        lines.push(String::new());
        lines.push(format!(
            "bullish score: {}/100. This is the balance of the signals above, not a forecast. Below 40 the \
             short-term tape is weak and the positive is on a longer horizon or on-chain: call token_progress, \
             token_holders or token_flow and lead with what you find there. The positives listed above are \
             still true.",
            read.bullish_score
        ));
        lines.join("\n")
    }
}

/// One of the zoom levels a post is ever actually about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Horizon {
    pub timeframe: Timeframe,
    pub bars: u32,
    pub label: &'static str,
}

/// Hourly, 4-hour, daily and weekly.
pub const HORIZONS: [Horizon; 4] = [
    Horizon {
        timeframe: Timeframe::H1,
        bars: 120,
        label: "hourly (last 5 days)",
    },
    Horizon {
        timeframe: Timeframe::H4,
        bars: 120,
        label: "4-hour (last 20 days)",
    },
    Horizon {
        timeframe: Timeframe::D1,
        bars: 120,
        label: "daily (last 4 months)",
    },
    Horizon {
        timeframe: Timeframe::W1,
        bars: 52,
        label: "weekly (the whole story)",
    },
];

/// One horizon's read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HorizonRead {
    pub timeframe: Timeframe,
    pub label: String,
    /// Bars that came back; zero when the fetch failed.
    pub bars: usize,
    /// `None` under 20 bars.
    pub read: Option<TechnicalRead>,
}

/// `chart_timeframes`: one timeframe is one opinion; this reads all four and
/// says which agree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChartTimeframesReport {
    pub horizons: Vec<HorizonRead>,
}

impl Report for ChartTimeframesReport {
    fn text(&self) -> String {
        let mut lines: Vec<String> = self
            .horizons
            .iter()
            .map(|horizon| {
                let Some(read) = &horizon.read else {
                    return format!(
                        "{}: only {} bars available, not enough to read.",
                        horizon.label, horizon.bars
                    );
                };
                let event = match read.event {
                    Some(event) => format!(
                        "  {} at ${}, {} bars ago{}",
                        event.kind.as_str(),
                        to_precision(event.level, 4),
                        event.bars_ago,
                        if event.volume_confirmed {
                            ", volume confirmed"
                        } else {
                            ", unconfirmed"
                        }
                    ),
                    None => "  no level crossed recently".into(),
                };
                let support = match read.levels.support {
                    Some(support) => format!(
                        "  support ${} ({} touches)",
                        to_precision(support, 4),
                        read.levels
                            .support_touches
                            .map(|n| n.to_string())
                            .unwrap_or_else(|| "undefined".into())
                    ),
                    None => "  no support below price on this timeframe".into(),
                };
                [
                    format!(
                        "{} [{}, {} bars]",
                        horizon.label, horizon.timeframe, read.bars
                    ),
                    format!(
                        "  {} {}%, trend {}, {}",
                        if read.change_pct >= 0.0 { "up" } else { "down" },
                        js_number(read.change_pct.abs()),
                        read.trend.as_str(),
                        read.structure
                    ),
                    event,
                    support,
                    format!("  bullish {}/100", read.bullish_score),
                    format!(
                        "  best positive: {}",
                        read.positives.first().map_or("none", String::as_str)
                    ),
                ]
                .join("\n")
            })
            .collect();

        let known: Vec<u32> = self
            .horizons
            .iter()
            .filter_map(|h| h.read.as_ref().map(|r| r.bullish_score))
            .collect();
        let up = known.iter().filter(|score| **score >= 50).count();
        let n = known.len();
        lines.push(String::new());
        lines.push(if n == 0 {
            "No timeframe had enough history to read.".into()
        } else if up == n {
            format!("All {n} timeframes read bullish. That agreement is itself the story.")
        } else if up == 0 {
            format!(
                "None of the {n} timeframes reads bullish on price. The positive is on-chain today: try \
                 token_holders or token_flow, and lead with what you find."
            )
        } else {
            format!(
                "{up} of {n} timeframes read bullish. Post about the horizon the day is actually about, and \
                 do not pretend the others agree with it."
            )
        });
        lines.join("\n")
    }
}

/// `token_progress`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressReport {
    /// Daily candles that came back.
    pub days_read: usize,
    /// `None` under two days.
    pub progress: Option<Progress>,
}

impl Report for ProgressReport {
    fn text(&self) -> String {
        let Some(progress) = &self.progress else {
            return "Not enough daily history to say anything about progress.".into();
        };
        let mut lines = progress.notes.clone();
        lines.push(String::new());
        lines.push(
            "This is the long horizon: where the token is against where it was, not what it did in the last \
             hour. It is what a holder who has been here for weeks is actually asking about."
                .into(),
        );
        lines.join("\n")
    }
}
