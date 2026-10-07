//! The closed sets some settings choose from, each spelled as in the legacy
//! `.env` (`SERVICE_ROLE=ingest`, `CHART_TIMEFRAME=4h`).

use std::fmt;
use std::str::FromStr;

/// The spellings a choice accepts, for the error that names them.
pub(crate) trait Choice: FromStr + Copy {
    /// Every accepted spelling, in declaration order.
    const SPELLINGS: &'static [&'static str];
}

macro_rules! choice {
    ($(#[$meta:meta])* $name:ident { $($(#[$vmeta:meta])* $variant:ident => $spelling:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name {
            $($(#[$vmeta])* $variant),+
        }

        impl $name {
            /// The spelling the environment uses.
            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $spelling),+
                }
            }
        }

        impl Choice for $name {
            const SPELLINGS: &'static [&'static str] = &[$($spelling),+];
        }

        impl FromStr for $name {
            type Err = ();

            fn from_str(value: &str) -> Result<Self, ()> {
                match value {
                    $($spelling => Ok(Self::$variant),)+
                    _ => Err(()),
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

choice! {
    /// `SERVICE_ROLE`: which jobs this process runs. `all` runs every role in
    /// one process; split them to scale or restart the reply side without
    /// pausing ingestion, which is the half with a deadline.
    ServiceRole {
        All => "all",
        Ingest => "ingest",
        Reply => "reply",
        Api => "api",
        Chart => "chart",
        Morning => "morning",
        Moon => "moon",
        Calendar => "calendar",
        Narrative => "narrative",
        Article => "article",
        Quote => "quote",
        Lore => "lore",
    }
}

choice! {
    /// `AGENT_MODE`: the tool-using agent or the single-shot drafter.
    AgentMode {
        Langgraph => "langgraph",
        Simple => "simple",
    }
}

choice! {
    /// `REPLY_MODE`: `worker` drains the queue continuously, so a webhook
    /// delivery becomes a reply in seconds; `cron` keeps the old timer.
    ReplyMode {
        Worker => "worker",
        Cron => "cron",
    }
}

choice! {
    /// `CHART_TIMEFRAME`: the candle size the chart agent reads.
    ChartTimeframe {
        M5 => "5m",
        M15 => "15m",
        H1 => "1h",
        H4 => "4h",
        H12 => "12h",
        D1 => "1d",
        W1 => "1w",
    }
}

choice! {
    /// `CHART_RENDERER`: `native` draws the chart from fetched candles;
    /// `screenshot` drives a headless browser at a page that draws one.
    ChartRenderer {
        Native => "native",
        Screenshot => "screenshot",
    }
}

choice! {
    /// `VIDEO_GEN_API`: `ladder` speaks `POST /video/generations` and polls the
    /// job; `openai` speaks Sora's `POST /videos`.
    VideoGenApi {
        Ladder => "ladder",
        OpenAi => "openai",
    }
}

choice! {
    /// One of `LORE_SEEDS`: where the lore post's seed comes from.
    LoreSeed {
        /// Exa, then Google News.
        News => "news",
        /// reddit and 4chan.
        Forum => "forum",
        /// A recall from memory.
        Memory => "memory",
    }
}
