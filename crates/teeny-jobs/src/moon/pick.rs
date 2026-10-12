//! Today's yardstick: one coin above the token on the board, in the band,
//! not one memory says was used lately. A port of `pickTarget`,
//! `usedSymbols` and `printable` (`legacy/src/moon/ladder.ts`,
//! `legacy/src/moon/pipeline.ts`); the rung and ladder arithmetic is
//! `teeny_media::moon`.

use regex::Regex;
use teeny_market::leaderboard::ListedCoin;
use teeny_media::moon::ListedCoin as DrawnCoin;

/// How the pick is bounded.
#[derive(Debug, Clone, PartialEq)]
pub struct PickOptions {
    /// Below this the target is a neighbour, not a moon.
    pub min_multiple: f64,
    /// Above this the number stops meaning anything to a reader.
    pub max_multiple: f64,
    /// Symbols posted recently; skipped unless nothing else is left.
    pub exclude: Vec<String>,
    /// In `[0, 1)`.
    pub random: f64,
}

impl Default for PickOptions {
    fn default() -> Self {
        Self {
            min_multiple: 2.0,
            max_multiple: 2_000.0,
            exclude: Vec::new(),
            random: 0.0,
        }
    }
}

/// One coin from the board, above the token, chosen at random within the
/// band. When every candidate was used lately the list resets rather than
/// the day being skipped; nothing above the token is `None`.
pub fn pick_target(
    token: &ListedCoin,
    board: &[ListedCoin],
    options: &PickOptions,
) -> Option<ListedCoin> {
    let multiple = |coin: &ListedCoin| coin.market_cap_usd / token.market_cap_usd;
    let above: Vec<&ListedCoin> = board
        .iter()
        .filter(|coin| coin.id != token.id && multiple(coin) >= options.min_multiple)
        .collect();
    if above.is_empty() {
        return None;
    }
    let in_band: Vec<&ListedCoin> = above
        .iter()
        .copied()
        .filter(|coin| multiple(coin) <= options.max_multiple)
        .collect();
    let pool = if in_band.is_empty() { above } else { in_band };
    let fresh: Vec<&ListedCoin> = pool
        .iter()
        .copied()
        .filter(|coin| !options.exclude.contains(&coin.symbol))
        .collect();
    let candidates = if fresh.is_empty() { pool } else { fresh };
    let index =
        ((options.random * candidates.len() as f64).floor() as usize).min(candidates.len() - 1);
    Some(candidates[index].clone())
}

/// The board's symbols that appear in recalled prose: whole tokens,
/// case-insensitive, with or without the cashtag.
pub fn used_symbols(recalled: &str, board: &[ListedCoin]) -> Vec<String> {
    if recalled.trim().is_empty() {
        return Vec::new();
    }
    let mut used: Vec<String> = Vec::new();
    for coin in board {
        let pattern = format!(
            r"(?i)(^|[^A-Za-z0-9_])\$?{}([^A-Za-z0-9_]|$)",
            regex::escape(&coin.symbol)
        );
        if Regex::new(&pattern).is_ok_and(|pattern| pattern.is_match(recalled))
            && !used.contains(&coin.symbol)
        {
            used.push(coin.symbol.clone());
        }
    }
    used
}

/// Whether a ticker can be drawn and read: letters, digits, `.`, `_`, `-`.
pub fn printable(symbol: &str) -> bool {
    let mut chars = symbol.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && symbol.chars().count() <= 16
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// A board row as the picture takes it.
pub fn drawn(coin: &ListedCoin) -> DrawnCoin {
    DrawnCoin {
        id: coin.id.clone(),
        symbol: coin.symbol.clone(),
        name: coin.name.clone(),
        price_usd: coin.price_usd,
        market_cap_usd: coin.market_cap_usd,
        circulating_supply: coin.circulating_supply,
        rank: coin.rank.map(|rank| rank as u32),
        board_rank: coin.board_rank.map(|rank| rank as u32),
        change_24h_pct: coin.change24h_pct,
    }
}

#[cfg(test)]
#[path = "pick_tests.rs"]
pub(crate) mod tests;
