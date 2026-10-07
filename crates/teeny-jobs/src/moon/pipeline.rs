//! "$TINY with the market cap of X", once a day: read the board, pick a
//! yardstick that has not been used lately, draw the picture from the
//! arithmetic, write two lines under it, gate them, post. A port of
//! `legacy/src/moon/pipeline.ts`.
//!
//! The picture is drawn here rather than generated, so every number on it is
//! one this service computed from CoinGecko's row a minute ago. Memory on
//! both sides: what the series said is recalled so a yardstick is not
//! repeated tomorrow, and what went out is written back with its ticker.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::json;
use teeny_market::leaderboard::{CoinGeckoLeaderboard, ListedCoin};
use teeny_media::moon::{Logo, MoonChartInput, Rung, ladder, render_moon_svg, rung};
use teeny_media::png::svg_to_png;
use teeny_media::reference::TextModel;
use teeny_memory::Memory;
use teeny_safety::gate::{SafetyDecision, SafetyGate};
use teeny_safety::house_style::{
    HouseStyleOptions, apply_house_style, asks_the_reader, count_emoji, without_the_ask,
};
use teeny_store::holds::{HoldContext, HoldKind, HoldSurface};
use teeny_store::journal::Outcome;
use teeny_x::MAX_POST_LENGTH;

use super::pick::{PickOptions, drawn, pick_target, printable, used_symbols};
use super::prompt::{MoonCaptionInput, moon_caption_prompt};
use crate::morning::pipeline::Mode;
use crate::morning::post::{Attachment, MimeType, OwnPost, OwnPoster, PostFailure};
use crate::morning::prompts::{Ask, StageContext};
use crate::morning::stages::{Caption, ask_json, js_len};

/// The board, one coin, and a logo.
#[async_trait]
pub trait LeaderboardSource: Send + Sync {
    async fn category(&self, id: &str, limit: usize) -> Result<Vec<ListedCoin>>;
    async fn coin(&self, id: &str) -> Result<ListedCoin>;
    async fn logo(&self, url: &str) -> Option<Logo>;
}

#[async_trait]
impl LeaderboardSource for CoinGeckoLeaderboard {
    async fn category(&self, id: &str, limit: usize) -> Result<Vec<ListedCoin>> {
        Ok(CoinGeckoLeaderboard::category(self, id, limit).await?)
    }
    async fn coin(&self, id: &str) -> Result<ListedCoin> {
        Ok(CoinGeckoLeaderboard::coin(self, id).await?)
    }
    async fn logo(&self, url: &str) -> Option<Logo> {
        CoinGeckoLeaderboard::logo(self, url)
            .await
            .map(|logo| Logo {
                data: logo.data,
                mime_type: logo.mime_type.to_owned(),
            })
    }
}

/// Everything the moon reads and writes through.
pub struct MoonPipeline {
    pub source: Arc<dyn LeaderboardSource>,
    pub text: Arc<dyn TextModel>,
    pub memory: Arc<dyn Memory>,
    pub poster: Arc<dyn OwnPoster>,
    pub safety: Option<Arc<SafetyGate>>,
    pub persona: String,
    pub bot_username: String,
    /// The account's own token, by CoinGecko id.
    pub coin_id: String,
    /// The category the yardsticks come from.
    pub category: String,
    /// How the category is named on the picture and in the caption.
    pub board_name: String,
    pub board_size: usize,
    /// How many rungs are drawn beside the target.
    pub rungs: usize,
    /// CoinGecko ids never used as a yardstick or a rung.
    pub exclude_ids: Vec<String>,
    pub min_multiple: f64,
    pub max_multiple: f64,
    pub max_emoji: usize,
    /// Where a copy of every picture is written.
    pub output_dir: Option<PathBuf>,
    pub watermark: Option<String>,
    pub random: f64,
    pub now: DateTime<Utc>,
}

/// What the moon did.
#[derive(Debug, Clone)]
pub struct MoonResult {
    pub token: ListedCoin,
    pub board: Vec<ListedCoin>,
    pub target: Rung,
    pub ladder: Vec<Rung>,
    /// Symbols memory says were posted lately and were therefore skipped.
    pub excluded: Vec<String>,
    pub svg: String,
    pub png: Vec<u8>,
    pub path: Option<PathBuf>,
    pub caption: String,
    pub posted_id: Option<String>,
    pub outcome: Outcome,
    pub problems: Vec<String>,
    pub history: String,
    pub community: String,
}

impl MoonPipeline {
    async fn recall(&self, query: &str) -> String {
        self.memory.recall(query, None).await.unwrap_or_default()
    }

    async fn logo(&self, coin: &ListedCoin) -> Option<Logo> {
        let url = coin.image_url.as_deref()?;
        self.source.logo(url).await
    }

    async fn ask(&self, ask: &Ask) -> Result<String> {
        Ok(
            ask_json::<Caption>(self.text.as_ref(), &ask.system, &ask.prompt)
                .await?
                .text,
        )
    }

    pub async fn run(&self, mode: Mode) -> Result<MoonResult> {
        let today = self.now.format("%Y-%m-%d").to_string();
        let mut problems = Vec::new();

        // 1. What this series has already said, and who it is for.
        let (history, community) = tokio::join!(
            self.recall(
                "Which \"market cap of\" moon-chart posts has this account published lately? For each: which coin was the yardstick (its ticker), the multiple, and the joke or framing it used."
            ),
            self.recall(
                "Who are the holders, builders and community members this account talks to? What do they care about, what have they been asking, and what is the mood right now?"
            ),
        );

        // 2. The board, with our own row on it.
        let board = self
            .source
            .category(&self.category, self.board_size)
            .await?;
        let token = match board.iter().find(|coin| coin.id == self.coin_id) {
            Some(token) => token.clone(),
            None => {
                let token = self.source.coin(&self.coin_id).await?;
                problems.push(format!(
                    "${} is not on the {} board ({} coins); compared against it from outside",
                    token.symbol,
                    self.board_name,
                    board.len()
                ));
                token
            }
        };

        // 3. The yardstick: above us, in the band, not used lately, not struck off.
        let eligible: Vec<ListedCoin> = board
            .iter()
            .filter(|coin| !self.exclude_ids.contains(&coin.id) && printable(&coin.symbol))
            .cloned()
            .collect();
        let excluded = used_symbols(&history, &eligible);
        let target = pick_target(
            &token,
            &eligible,
            &PickOptions {
                min_multiple: self.min_multiple,
                max_multiple: self.max_multiple,
                exclude: excluded.clone(),
                random: self.random,
            },
        )
        .ok_or_else(|| {
            anyhow!(
                "nothing on the {} board sits above ${}; there is no moon to draw",
                self.board_name,
                token.symbol
            )
        })?;
        let drawn_token = drawn(&token);
        let drawn_board: Vec<_> = eligible.iter().map(drawn).collect();
        let headline = rung(&drawn_token, &drawn(&target));
        let climb = ladder(
            &drawn_token,
            &drawn_board,
            &drawn(&target),
            self.rungs,
            self.min_multiple,
        );
        tracing::info!(
            target = target.symbol,
            multiple = headline.multiple,
            ?excluded,
            "moon: target picked"
        );

        // 4. The picture. A missing logo is a disc with a letter.
        let (token_logo, target_logo) = tokio::join!(self.logo(&token), self.logo(&target));
        let svg = render_moon_svg(&MoonChartInput {
            token: drawn_token.clone(),
            target: headline.clone(),
            ladder: climb.clone(),
            board_name: self.board_name.clone(),
            board_size: Some(board.len() as u32),
            date: today.clone(),
            token_logo,
            target_logo,
            watermark: self.watermark.clone(),
            width: None,
            height: None,
        });
        let png = svg_to_png(&svg)?;
        let path = match &self.output_dir {
            Some(dir) => {
                let name = format!(
                    "{}-vs-{}-{}.png",
                    token.symbol,
                    target
                        .symbol
                        .chars()
                        .filter(char::is_ascii_alphanumeric)
                        .collect::<String>(),
                    self.now.timestamp_millis()
                );
                let path = dir.join(name);
                std::fs::create_dir_all(dir)?;
                std::fs::write(&path, &png)?;
                Some(path)
            }
            None => None,
        };

        // 5. The caption, then the filter.
        let context = StageContext {
            persona: self.persona.clone(),
            bot_username: self.bot_username.clone(),
            today,
        };
        let mut caption = self
            .caption(
                &context,
                &MoonCaptionInput {
                    token: &drawn_token,
                    target: &headline,
                    ladder: &climb,
                    board_name: &self.board_name,
                    board_size: board.len(),
                    history: &history,
                    community: &community,
                    max_emoji: self.max_emoji,
                    max_length: MAX_POST_LENGTH,
                },
            )
            .await?;
        let mut held = false;
        if let Some(safety) = &self.safety {
            let outgoing = safety
                .outbound(
                    &caption,
                    Some(&HoldContext {
                        kind: Some(HoldKind::Draft),
                        tool: Some("moon_post".into()),
                        ..HoldContext::new(HoldSurface::Moon)
                    }),
                )
                .await;
            if !outgoing.fixes.is_empty() {
                problems.push(format!(
                    "the content filter edited the caption: {}",
                    outgoing.fixes.join("; ")
                ));
            }
            match outgoing.decision {
                SafetyDecision::Clean => caption = outgoing.text,
                SafetyDecision::Held(hold) => {
                    held = true;
                    problems.push(format!(
                        "content filter withheld the caption: {}",
                        hold.category
                    ));
                }
            }
        }

        let mut result = MoonResult {
            token,
            board,
            target: headline,
            ladder: climb,
            excluded,
            svg,
            png,
            path,
            caption,
            posted_id: None,
            outcome: Outcome::DryRun,
            problems,
            history,
            community,
        };

        // 6. Post, or say what would have been posted.
        if held {
            result.caption = String::new();
            result.outcome = Outcome::Held;
            return Ok(result);
        }
        if mode == Mode::Live {
            let post = OwnPost {
                kind: "moon".into(),
                text: result.caption.clone(),
                media: Some(Attachment {
                    data: result.png.clone(),
                    mime_type: MimeType::Png,
                }),
                quote: None,
            };
            match self.poster.post(post).await {
                Ok(posted) if posted.dry_run => {}
                Ok(posted) => {
                    result.outcome = Outcome::Posted;
                    result.caption = posted.text;
                    result.posted_id = Some(posted.id);
                }
                Err(PostFailure::Held(reason)) => {
                    result.problems.push(reason);
                    result.outcome = Outcome::Skipped;
                    return Ok(result);
                }
                Err(PostFailure::Refused(reason)) => {
                    result
                        .problems
                        .push(format!("content filter withheld the caption: {reason}"));
                    result.caption = String::new();
                    result.outcome = Outcome::Held;
                    return Ok(result);
                }
                Err(PostFailure::Failed(error)) => return Err(anyhow!(error)),
            }
        }
        self.capture(&mut result).await;
        Ok(result)
    }

    async fn caption(
        &self,
        context: &StageContext,
        input: &MoonCaptionInput<'_>,
    ) -> Result<String> {
        let ask = moon_caption_prompt(context, input);
        let mut text = self.ask(&ask).await?;
        if asks_the_reader(&text) {
            // The post ends on the aside, not on an ask.
            let retry = self
                .ask(&Ask {
                    system: ask.system.clone(),
                    prompt: format!(
                        "{}\n\nYour draft:\n{text}\n\nIt ends by asking the reader something. Rewrite it to end on the aside, with no question in it, keeping everything else.",
                        ask.prompt
                    ),
                })
                .await?;
            text = if asks_the_reader(&retry) {
                without_the_ask(&retry)
            } else {
                retry
            };
        }
        if js_len(&text) > MAX_POST_LENGTH || count_emoji(&text) > self.max_emoji {
            text = self
                .ask(&Ask {
                    system: ask.system.clone(),
                    prompt: format!(
                        "{}\n\nYour draft was {} characters with {} emoji:\n{text}\n\nRewrite it under {MAX_POST_LENGTH} characters with at most {} emoji.",
                        ask.prompt,
                        js_len(&text),
                        count_emoji(&text),
                        self.max_emoji
                    ),
                })
                .await?;
        }
        Ok(apply_house_style(
            &text,
            &HouseStyleOptions {
                max_emoji: self.max_emoji,
                max_length: MAX_POST_LENGTH,
                replying_to: None,
            },
        )
        .text)
    }

    /// What went out, with the yardstick's ticker in it, because that is
    /// what tomorrow's exclusion list is read from.
    async fn capture(&self, result: &mut MoonResult) {
        let id = result
            .posted_id
            .clone()
            .unwrap_or_else(|| format!("dry-run:{}", self.now.timestamp_millis()));
        let mut payload = json!({
            "text": result.caption,
            "token": result.token.symbol,
            "yardstick": result.target.coin.symbol,
            "yardstickName": result.target.coin.name,
            "multiple": (result.target.multiple * 100.0).round() / 100.0,
            "impliedPriceUsd": result.target.implied_price_usd,
            "marketCapUsd": result.token.market_cap_usd,
            "published": result.outcome == Outcome::Posted,
        });
        if let Some(rank) = result.token.board_rank {
            payload["boardRank"] = json!(rank);
        }
        if let Err(error) = self
            .memory
            .capture_activity("moon_post", &id, payload, None)
            .await
        {
            result
                .problems
                .push(format!("could not remember the post: {error}"));
        }
    }
}

#[cfg(test)]
#[path = "pipeline_tests.rs"]
mod tests;
