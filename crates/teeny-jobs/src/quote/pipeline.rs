//! One quote post a day: somebody bullish, lifted onto the timeline with a
//! line over it. A port of `legacy/src/quote/pipeline.ts`.
//!
//! The judgement is one call (which post, and what to say); everything
//! around it is a rule: two searches, the filter, the log of what was
//! quoted, the content gate, the post. The log is what makes the series
//! safe on a timer: a post is quoted once, ever, and an author rests a week.

use std::sync::Arc;

use anyhow::{Result, anyhow};
use chrono::{DateTime, TimeDelta, Utc};
use regex::Regex;
use serde_json::json;
use teeny_core::types::PostId;
use teeny_media::reference::TextModel;
use teeny_memory::Memory;
use teeny_safety::gate::{SafetyDecision, SafetyGate};
use teeny_safety::house_style::{HouseStyleOptions, apply_house_style, count_emoji};
use teeny_safety::screen::Direction;
use teeny_store::holds::{HoldContext, HoldKind, HoldSurface};
use teeny_store::journal::Outcome;
use teeny_x::MAX_POST_LENGTH;
use teeny_x::cache::XReads;
use teeny_x::model::XPost;

use super::candidates::{Candidate, CandidateRules, Screen, Skipped, find_candidates};
pub use super::log::{QuoteLog, QuotedPost};
use super::prompt::{QuotePick, quote_prompt};
pub use crate::morning::pipeline::Mode;
use crate::morning::post::{OwnPost, OwnPoster, PostFailure};
use crate::morning::prompts::{Ask, StageContext};
use crate::morning::stages::{ask_json, js_len};

/// Everything the quote reads and writes through.
pub struct QuotePipeline {
    pub x: Arc<dyn XReads>,
    pub text: Arc<dyn TextModel>,
    pub memory: Arc<dyn Memory>,
    pub log: Arc<dyn QuoteLog>,
    pub poster: Arc<dyn OwnPoster>,
    /// The lexicon over every candidate, and the full gate over the words.
    pub safety: Option<Arc<SafetyGate>>,
    pub persona: String,
    pub bot_username: String,
    pub trusted_handles: Vec<String>,
    pub own_token_addresses: Vec<String>,
    pub own_symbol: String,
    /// The operator's ticker query, X search syntax.
    pub query: String,
    pub max_age: TimeDelta,
    pub author_cooldown: TimeDelta,
    /// How many posts each search reads.
    pub max_results: u32,
    /// How many survivors the picker sees.
    pub keep: usize,
    pub max_emoji: usize,
    pub now: DateTime<Utc>,
}

/// What the quote did.
#[derive(Debug, Clone)]
pub struct QuoteResult {
    pub candidates: Vec<Candidate>,
    pub skipped: Vec<Skipped>,
    pub searched: usize,
    pub chosen: Option<XPost>,
    pub why: String,
    pub text: String,
    pub posted_id: Option<String>,
    pub outcome: Outcome,
    pub problems: Vec<String>,
    pub recalled: String,
}

impl QuotePipeline {
    async fn ask(&self, ask: &Ask) -> Result<QuotePick> {
        Ok(ask_json(self.text.as_ref(), &ask.system, &ask.prompt).await?)
    }

    pub async fn run(&self, mode: Mode) -> Result<QuoteResult> {
        let today = self.now.format("%Y-%m-%d").to_string();
        let mut problems = Vec::new();

        // 1. What this series has said, and what it has already quoted.
        let (history, quoted) = tokio::join!(
            async {
                self.memory
                    .recall(
                        "Which posts by other people has this account quote-posted lately, and what did it say over each? The author, the gist of their post, and the words the account put over it.",
                        None,
                    )
                    .await
                    .unwrap_or_default()
            },
            self.log.list(),
        );
        let quoted = quoted?;

        // 2. The candidates.
        let screen: Option<Screen> = self.safety.as_ref().map(|safety| {
            let safety = Arc::clone(safety);
            Arc::new(move |text: &str| safety.screen(text, Direction::Inbound)) as Screen
        });
        let rules = CandidateRules {
            bot_username: self.bot_username.clone(),
            trusted_handles: self.trusted_handles.clone(),
            own_token_addresses: self.own_token_addresses.clone(),
            own_symbol: self.own_symbol.clone(),
            quoted,
            max_age: self.max_age,
            author_cooldown: self.author_cooldown,
            screen,
            now: self.now,
        };
        let found = find_candidates(self.x.as_ref(), &self.query, self.max_results, self.keep, &rules).await;
        tracing::info!(searched = found.searched, kept = found.candidates.len(), skipped = found.skipped.len(), "quote: candidates found");
        let mut result = QuoteResult {
            candidates: found.candidates,
            skipped: found.skipped,
            searched: found.searched,
            chosen: None,
            why: String::new(),
            text: String::new(),
            posted_id: None,
            outcome: Outcome::Skipped,
            problems: Vec::new(),
            recalled: history.clone(),
        };
        if result.candidates.is_empty() {
            result.why = "nothing recent survived the rules".into();
            return Ok(result);
        }

        // 3. The choice and the words, one call.
        let ask = quote_prompt(
            &StageContext {
                persona: self.persona.clone(),
                bot_username: self.bot_username.clone(),
                today,
            },
            self.now,
            &result.candidates,
            &history,
            self.max_emoji,
            MAX_POST_LENGTH,
        );
        let mut pick = self.ask(&ask).await?;
        let index = pick.pick.unwrap_or(0) as usize;
        if index < 1 || index > result.candidates.len() {
            result.why = pick.why;
            return Ok(result);
        }
        if js_len(&pick.text) > MAX_POST_LENGTH
            || count_emoji(&pick.text) > self.max_emoji
            || pick.text.trim().is_empty()
        {
            let retry = self
                .ask(&Ask {
                    system: ask.system.clone(),
                    prompt: format!(
                        "{}\n\nYou picked {index} and wrote {} characters with {} emoji:\n{}\n\nKeep the pick. Rewrite the text under {MAX_POST_LENGTH} characters with at most {} emoji, and not empty.",
                        ask.prompt,
                        js_len(&pick.text),
                        count_emoji(&pick.text),
                        pick.text,
                        self.max_emoji
                    ),
                })
                .await?;
            pick = QuotePick {
                pick: Some(index as u32),
                ..retry
            };
        }
        let chosen = result.candidates[index - 1].post.clone();
        let mut text = apply_house_style(
            &pick.text,
            &HouseStyleOptions {
                max_emoji: self.max_emoji,
                max_length: MAX_POST_LENGTH,
                replying_to: None,
            },
        )
        .text;
        // The handle is on the quoted post already; in the text it is a tag.
        if let Some(author) = &chosen.author_username
            && let Ok(handle) = Regex::new(&format!(r"(?i)@{}\b", regex::escape(author)))
        {
            let stripped = handle.replace_all(&text, "");
            let spaces = Regex::new(r"[ \t]{2,}").expect("valid");
            text = spaces.replace_all(&stripped, " ").trim().to_owned();
        }
        result.why = pick.why;
        result.chosen = Some(chosen.clone());
        if text.is_empty() {
            problems.push("the picker chose a post and wrote nothing over it".to_owned());
            result.problems = problems;
            return Ok(result);
        }

        // 4. The filter, on the words that go out.
        if let Some(safety) = &self.safety {
            let outgoing = safety
                .outbound(
                    &text,
                    Some(&HoldContext {
                        kind: Some(HoldKind::Draft),
                        post_id: Some(chosen.id.to_string()),
                        author_username: chosen.author_username.clone(),
                        ..HoldContext::new(HoldSurface::Quote)
                    }),
                )
                .await;
            if !outgoing.fixes.is_empty() {
                problems.push(format!(
                    "the content filter edited the text: {}",
                    outgoing.fixes.join("; ")
                ));
            }
            match outgoing.decision {
                SafetyDecision::Clean => text = outgoing.text,
                SafetyDecision::Held(hold) => {
                    problems.push(format!("content filter withheld the text: {}", hold.category));
                    result.problems = problems;
                    result.outcome = Outcome::Held;
                    return Ok(result);
                }
            }
        }
        result.problems = problems;
        result.text = text;
        result.outcome = Outcome::DryRun;

        // 5. Post, or say what would have been posted. The log is written on
        // a rehearsal too, so it does not pick the same post twice.
        if mode == Mode::Live {
            let post = OwnPost {
                kind: "quote".into(),
                text: result.text.clone(),
                media: None,
                quote: Some(PostId::from(chosen.id.as_str())),
            };
            match self.poster.post(post).await {
                Ok(posted) if posted.dry_run => {}
                Ok(posted) => {
                    result.outcome = Outcome::Posted;
                    result.text = posted.text;
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
                        .push(format!("content filter withheld the text: {reason}"));
                    result.text = String::new();
                    result.outcome = Outcome::Held;
                    return Ok(result);
                }
                Err(PostFailure::Failed(error)) => return Err(anyhow!(error)),
            }
        }
        self.remember(&mut result).await;
        Ok(result)
    }

    /// The log first, since it is the rule; memory second, since it is the voice.
    async fn remember(&self, result: &mut QuoteResult) {
        let Some(chosen) = result.chosen.clone() else {
            return;
        };
        let entry = QuotedPost {
            post_id: chosen.id.to_string(),
            author_username: chosen.author_username.clone(),
            quoted_at: teeny_store::iso(self.now),
            own_post_id: result.posted_id.clone(),
        };
        if let Err(error) = self.log.record(entry).await {
            result.problems.push(format!("could not log the quote: {error}"));
        }
        let id = result
            .posted_id
            .clone()
            .unwrap_or_else(|| format!("dry-run:{}", self.now.timestamp_millis()));
        let payload = json!({
            "text": result.text,
            "quotedPostId": chosen.id.as_str(),
            "quotedAuthor": chosen.author_username,
            "quotedText": chosen.text,
            "why": result.why,
            "published": result.outcome == Outcome::Posted,
        });
        if let Err(error) = self
            .memory
            .capture_activity("quote_post", &id, payload, None)
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
