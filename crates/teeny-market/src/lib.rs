//! Market data (DexScreener, GeckoTerminal, GMGN, Solana, CoinGecko),
//! technical analysis and the data behind the read-only market tools.
//!
//! A port of `legacy/src/market`, `legacy/src/ta` and the data side of the
//! market, community and news tools. Numbers and the text handed to the model
//! match the TypeScript: `fixtures/golden` holds what the legacy code returned
//! on fixed inputs, and the tests compare against it.
//!
//! - Providers: [`dexscreener`], [`geckoterminal`], [`gmgn`], [`solana`],
//!   [`global`] (CoinGecko global) and [`leaderboard`] (CoinGecko markets, for
//!   the moon board), over [`http`] with the per-provider [`limiter`].
//! - [`market::TokenMarket`] merges them, caches, and folds weekly candles.
//! - [`ta`]: the indicators, the chart read and the long horizon.
//! - [`reports::MarketReports`]: one method per read-only market tool, each
//!   rendering the legacy tool text; [`community`] and [`news`] do the same
//!   for `community_pulse` and `crypto_news` behind a search trait.
//! - [`format`](mod@format): numbers printed the way JavaScript printed them.

pub mod community;
pub mod dexscreener;
mod error;
pub mod format;
pub mod geckoterminal;
pub mod global;
pub mod gmgn;
pub mod http;
pub mod leaderboard;
pub mod limiter;
pub mod market;
pub mod news;
pub mod reports;
pub mod solana;
pub mod ta;
pub mod types;

pub use error::{MarketError, Result};
pub use global::{GlobalMarketClient, GlobalSource};
pub use market::{MarketData, MarketOptions, TokenMarket};
pub use reports::{MarketReports, Report};
pub use types::{Candle, Timeframe, TokenOverview};

#[cfg(test)]
mod test_support;
