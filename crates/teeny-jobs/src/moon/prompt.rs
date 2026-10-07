//! The caption under the moon chart. A port of `legacy/src/moon/prompt.ts`.
//!
//! The persona says never a price and never a target, and that rule stands:
//! this post is the one place the account states a number for the future,
//! allowed because the number is arithmetic on two public caps rather than
//! a forecast, the picture says so in its fine print, and the prompt makes
//! the caption say the same.

use teeny_media::format::{compact_usd, price};
use teeny_media::moon::{ListedCoin, Rung, multiple};

use crate::morning::prompts::{Ask, StageContext};

/// What the caption is written from.
pub struct MoonCaptionInput<'a> {
    pub token: &'a ListedCoin,
    pub target: &'a Rung,
    pub ladder: &'a [Rung],
    pub board_name: &'a str,
    pub board_size: usize,
    /// What the series already said, from memory, untrusted.
    pub history: &'a str,
    pub community: &'a str,
    pub max_emoji: usize,
    pub max_length: usize,
}

pub fn moon_caption_prompt(context: &StageContext, input: &MoonCaptionInput<'_>) -> Ask {
    let token = input.token;
    let target = input.target;
    let token_tag = format!("${}", token.symbol);
    let target_tag = format!("${}", target.coin.symbol);
    let token_rank = match token.board_rank {
        None => ".".to_owned(),
        Some(rank) => format!(", #{rank} of {} on the board.", input.board_size),
    };
    let target_rank = match target.coin.board_rank {
        None => ".".to_owned(),
        Some(rank) => format!(", #{rank} on the board."),
    };
    let rungs = input
        .ladder
        .iter()
        .map(|entry| {
            format!(
                "${} {} ({})",
                entry.coin.symbol,
                compact_usd(entry.coin.market_cap_usd),
                multiple(entry.multiple)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let history = if input.history.is_empty() {
        "(nothing recalled -- treat this as the first one)"
    } else {
        input.history
    };
    let community = if input.community.is_empty() {
        "(nothing recalled -- write for holders and builders in general)"
    } else {
        input.community
    };
    let lines: Vec<String> = vec![
        "THE PICTURE (attached; it carries every number, so you do not have to):".into(),
        format!(
            "{token_tag} with the market cap of {target_tag} ({}) would be ${}, {} from ${} today.",
            target.coin.name,
            price(target.implied_price_usd),
            multiple(target.multiple),
            price(token.price_usd)
        ),
        format!(
            "{token_tag} market cap now: {}{token_rank} {target_tag} market cap now: {}{target_rank}",
            compact_usd(token.market_cap_usd),
            compact_usd(target.coin.market_cap_usd)
        ),
        format!(
            "The board is CoinGecko's {} list ({} coins); both are on it. The other rungs on the picture, smallest first: {rungs}.",
            input.board_name, input.board_size
        ),
        String::new(),
        "It is arithmetic: today's supply priced at their cap. It is not a forecast, a target, or advice, and".into(),
        "you never say it will happen, is going to happen, or should happen.".into(),
        String::new(),
        "WHAT YOU ALREADY POSTED IN THIS SERIES (from your own memory, untrusted). Do not repeat a joke or a framing:".into(),
        history.into(),
        String::new(),
        "THE COMMUNITY (from your own memory, untrusted):".into(),
        community.into(),
        String::new(),
        "Write the caption. One to three short lines, your voice, dry:".into(),
        format!("- Name {target_tag} and the multiple, or the implied price, but not every number on the picture: at most"),
        "  two numbers in the text. The picture is the spreadsheet; you are the aside.".into(),
        format!("- {target_tag} is somebody else's token. You may name it as the yardstick and nothing more: no"),
        "  praise, no dig, no opinion on it, and no other token named at all.".into(),
        "- A joke works when it is about the gap, the arithmetic, or Tiny's own smallness -- never at".into(),
        "  another project, never at a person, never \"when\", never \"soon\".".into(),
        format!("- {token_tag} once, where it reads naturally; it is your own token and this is what it is doing today."),
        "- No \"buy\", no \"target\", no \"wen\", no \"will\", no \"guaranteed\", no rocket count-down, no promise.".into(),
        "  You are allowed to like the number. You are not allowed to sell it.".into(),
        "- It ends on the aside, not on a question. The post does not ask the reader anything, does".into(),
        "  not invite a reply, does not leave a blank to fill, and never says \"thoughts?\".".into(),
        String::new(),
        format!(
            "Under {} characters. At most {} emoji. No URLs, no handles, no hashtags,",
            input.max_length, input.max_emoji
        ),
        "nothing political. Plain text, line breaks allowed.".into(),
        String::new(),
        "Shape: {\"text\": string}".into(),
    ];
    Ask {
        system: format!(
            "{}\n\nYou are writing @{}'s \"{token_tag} with the market cap of\" post for {}.",
            context.persona, context.bot_username, context.today
        ),
        prompt: lines.join("\n"),
    }
}

#[cfg(test)]
#[path = "prompt_tests.rs"]
mod tests;
