//! Everything a reply or chat turn starts with, gathered in one place. A
//! port of `legacy/src/memory/context.ts`.
//!
//! Before this, each harness asked memory two questions and took whatever
//! came back, including nothing: a recall that timed out read exactly like a
//! person the account had never met, and the drafter said so. Now:
//!
//! - What can be read exactly is read exactly. The account's own posts are in
//!   the post ledger and its answers to a person are in the mention queue;
//!   asking a semantic index for either is asking it to guess at a list that
//!   already exists.
//! - Every source fails on its own. Memory being down costs the turn its
//!   recalled context, not the ledger or the exchange history, and the
//!   reverse.
//! - A failure is said out loud. The drafter is told memory was unavailable,
//!   so it does not claim to remember nothing, and the journal records which
//!   source dropped out.

use chrono::{DateTime, TimeDelta, Utc};
use teeny_core::config::MemoryConfig;
use teeny_memory::text::clip;
use teeny_memory::{Memory, Person};
use teeny_store::posts::{OwnPost, PostLedger};
use teeny_store::{Exchange, ExchangeHistory, iso};

use crate::ctx::JobCtx;
use crate::journal_glue::summarise_recall;

/// The question that stands in for the ledger when there is none to read.
pub const OWN_ACTIVITY_QUERY: &str = "What has this account itself posted recently? Include its daily chart posts, the \
market reads it published, and anything it has said publicly in the last few days.";

/// The heading the account's own recent posts go under.
pub const OWN_ACTIVITY_HEADING: &str = "WHAT YOU YOURSELF HAVE POSTED RECENTLY:";

/// What the drafter is told when memory could not be read this turn.
pub const MEMORY_UNAVAILABLE: &str = "MEMORY WAS UNAVAILABLE THIS TURN. Nothing above or below says you have never met this \
person or never discussed this; you simply could not look. Do not claim either way.";

/// How much each exact source reads. Zero turns a source off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnLimits {
    /// The last answered mentions from the person.
    pub exchanges: usize,
    /// How far back the account's own posts are read.
    pub own_posts_hours: u32,
    /// How many of them.
    pub own_posts_limit: usize,
}

impl Default for TurnLimits {
    fn default() -> Self {
        Self {
            exchanges: 6,
            own_posts_hours: 72,
            own_posts_limit: 8,
        }
    }
}

impl TurnLimits {
    /// `MEMORY_EXCHANGES`, `MEMORY_OWN_POSTS_HOURS`, `MEMORY_OWN_POSTS_LIMIT`.
    pub fn from_config(config: &MemoryConfig) -> Self {
        Self {
            exchanges: config.exchanges as usize,
            own_posts_hours: config.own_posts_hours,
            own_posts_limit: config.own_posts_limit as usize,
        }
    }
}

/// What a turn is told before it is drafted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TurnMemory {
    /// The text the drafter reads as its memory.
    pub text: String,
    /// A few lines for the journal: what was actually recalled, not a
    /// paraphrase.
    pub recalled: Vec<String>,
    /// Sources that failed or timed out this turn, by name. Empty is a whole
    /// turn.
    pub degraded: Vec<String>,
}

/// What the turn is about.
#[derive(Debug, Clone, Copy, Default)]
pub struct TurnQuery<'q> {
    /// What memory is asked.
    pub query: &'q str,
    pub person: Option<&'q Person>,
    /// The X user id, when the turn answers a mention: reads the exact
    /// exchange history.
    pub author_id: Option<&'q str>,
    /// The post being answered, kept out of its own history.
    pub post_id: Option<&'q str>,
}

/// The sources a turn reads.
pub struct TurnContext<'a> {
    pub memory: &'a dyn Memory,
    /// The account's own timeline, read exactly. Absent, memory is asked.
    pub own_posts: Option<&'a dyn PostLedger>,
    /// The last answered exchanges with an X user, read exactly.
    pub exchanges: Option<&'a dyn ExchangeHistory>,
    pub limits: TurnLimits,
    pub now: DateTime<Utc>,
}

/// Wall-clock time the way the drafter reads it: to the minute, in UTC.
fn stamp(iso: &str) -> String {
    let minute: String = iso.chars().take(16).collect();
    format!("{} UTC", minute.replacen('T', " ", 1))
}

fn render_own_post(post: &OwnPost) -> String {
    format!(
        "[{} · {}] {}",
        stamp(&post.posted_at),
        post.kind,
        clip(post.text.as_deref().unwrap_or_default(), 400)
    )
}

impl<'a> TurnContext<'a> {
    /// The sources the job context holds: its memory, and when the
    /// conversation side is wired, the post ledger and the mention queue's
    /// exchange history; the limits from `MEMORY_*`.
    pub fn for_ctx(ctx: &'a JobCtx) -> Self {
        let conversation = ctx.conversation.as_deref();
        Self {
            memory: ctx.deps.memory.as_ref(),
            own_posts: conversation.map(|conversation| conversation.posts.as_ref()),
            exchanges: conversation.and_then(|conversation| conversation.exchanges.as_deref()),
            limits: TurnLimits::from_config(&ctx.config.memory),
            now: ctx.now(),
        }
    }

    /// Reads every source at once, each failing on its own.
    pub async fn gather(&self, input: TurnQuery<'_>) -> TurnMemory {
        let (exchanges, recalled, own) = tokio::join!(
            self.exchanges_with(input.author_id, input.post_id),
            self.memory.recall(input.query, input.person),
            self.own_activity(),
        );
        let mut degraded = Vec::new();
        let mut failed = |source: &str, error: &dyn std::fmt::Display| {
            tracing::warn!(%error, source, "turn context: {source} unavailable");
            degraded.push(source.to_owned());
        };
        let exchanges = exchanges.unwrap_or_else(|error| {
            failed("exchanges", &error);
            Vec::new()
        });
        let recalled = recalled.map_err(|error| failed("memory", &error)).ok();
        let own = own.unwrap_or_else(|error| {
            failed("own posts", &error);
            None
        });

        let handle = input
            .person
            .and_then(Person::handle_label)
            .unwrap_or_else(|| "this person".to_owned());
        let mut sections = Vec::new();
        if !exchanges.is_empty() {
            let mut lines = vec![format!(
                "YOUR LAST {} EXCHANGES WITH {handle}, newest first, exactly as they happened:",
                exchanges.len()
            )];
            lines.extend(exchanges.iter().map(|exchange| {
                format!(
                    "[{}] {handle}: {}\n  you: {}",
                    stamp(&exchange.at),
                    clip(&exchange.incoming, 280),
                    clip(&exchange.reply, 280)
                )
            }));
            sections.push(lines.join("\n"));
        }
        match &recalled {
            None => sections.push(MEMORY_UNAVAILABLE.to_owned()),
            Some(recalled) if !recalled.trim().is_empty() => {
                sections.push(recalled.trim().to_owned());
            }
            Some(_) => {}
        }
        if let Some(own) = own.as_deref().filter(|own| !own.trim().is_empty()) {
            sections.push(format!("{OWN_ACTIVITY_HEADING}\n{}", own.trim()));
        }

        // Deduplicated: memory often answers both questions with the same
        // general context when it knows little about the person.
        let mut lines = Vec::new();
        if !exchanges.is_empty() {
            lines.push(format!(
                "{} earlier exchanges with {handle}",
                exchanges.len()
            ));
        }
        lines.extend(summarise_recall(recalled.as_deref().unwrap_or_default(), 3));
        lines.extend(summarise_recall(own.as_deref().unwrap_or_default(), 2));
        lines.extend(
            degraded
                .iter()
                .map(|source| format!("({source} unavailable)")),
        );
        let mut recalled_lines: Vec<String> = Vec::new();
        for line in lines {
            if !recalled_lines.contains(&line) {
                recalled_lines.push(line);
            }
        }
        TurnMemory {
            text: sections.join("\n\n"),
            recalled: recalled_lines,
            degraded,
        }
    }

    async fn exchanges_with(
        &self,
        author_id: Option<&str>,
        post_id: Option<&str>,
    ) -> anyhow::Result<Vec<Exchange>> {
        match (author_id, self.exchanges) {
            (Some(author_id), Some(history)) if self.limits.exchanges > 0 => Ok(history
                .exchanges_with(author_id, self.limits.exchanges, post_id)
                .await?),
            _ => Ok(Vec::new()),
        }
    }

    /// The ledger when there is one, because it is the list itself; memory
    /// when there is not, which is how every turn read it before.
    async fn own_activity(&self) -> anyhow::Result<Option<String>> {
        let limits = self.limits;
        if limits.own_posts_limit == 0 || limits.own_posts_hours == 0 {
            return Ok(None);
        }
        let Some(ledger) = self.own_posts else {
            return Ok(Some(self.memory.recall(OWN_ACTIVITY_QUERY, None).await?));
        };
        let since = iso(self.now - TimeDelta::hours(i64::from(limits.own_posts_hours)));
        let posts = ledger.since(&since, limits.own_posts_limit).await?;
        Ok(Some(
            posts
                .iter()
                .filter(|post| {
                    post.text
                        .as_deref()
                        .is_some_and(|text| !text.trim().is_empty())
                })
                .map(render_own_post)
                .collect::<Vec<_>>()
                .join("\n"),
        ))
    }
}

#[cfg(test)]
#[path = "turn_context_tests.rs"]
mod tests;
