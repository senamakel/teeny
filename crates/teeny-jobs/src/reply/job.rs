//! The reply job: drains the mention queue, one batch a run. Never talks to
//! X's search. A port of `legacy/src/jobs/reply-job.ts`.
//!
//! Per mention, in the legacy order: refuse the account's own post, like it,
//! the reply budget, the content filter inbound, the draft (a public
//! `teeny-reply` turn with no write tool), house style, the content filter
//! outbound, the attachment, the pacing gap, then host code posts it, keeps
//! the exchange, marks the queue, the people record and the journal. A
//! failure marks the mention failed; nothing retries it.

use std::hash::BuildHasher;
use std::sync::Mutex;
use std::time::Duration;

use teeny_agent::deps::QueuedXPost;
use teeny_cadence::budget::{Assessment, Mention};
use teeny_core::types::UserId;
use teeny_safety::gate::SafetyDecision;
use teeny_safety::house_style::{HouseStyleOptions, apply_house_style};
use teeny_store::holds::{HoldContext, HoldSurface};
use teeny_store::journal::{Outcome, ReflectionInput, Tone, ToolCall, Trigger};
use teeny_store::mentions::{QueuedMention, Reply, ReplyLane};
use teeny_store::people::{Interaction, Observation};
use teeny_x::model::{Media, MediaType, XPost};

use super::draft::ReplyDraft;
use super::harness::{generate, is_sour, remember};
use crate::ctx::{Conversation, JobCtx, JobReport};
use crate::journal_glue::record_quietly;

/// The job's name in reports.
pub const JOB: &str = "reply";

/// The gap between two published replies, random within
/// `REPLY_MIN_INTERVAL_MS..=REPLY_MAX_INTERVAL_MS`: a fixed gap is its own
/// signature. Never below the minimum, whatever the range says.
pub fn gap_ms(min: u64, max: u64, roll: f64) -> u64 {
    if min == 0 {
        return 0;
    }
    let max = max.max(min);
    (min as f64 + roll * (max - min) as f64).round() as u64
}

/// A roll in `0.0..=1.0` from the standard library's random hasher keys.
fn random_roll() -> f64 {
    let seed = std::collections::hash_map::RandomState::new().hash_one(std::time::Instant::now());
    seed as f64 / u64::MAX as f64
}

/// The pacing brake, kept across runs: when the last reply went out.
pub struct Pacing {
    last: Mutex<Option<tokio::time::Instant>>,
    roll: Box<dyn Fn() -> f64 + Send + Sync>,
}

impl Default for Pacing {
    fn default() -> Self {
        Self::with_roll(random_roll)
    }
}

impl Pacing {
    /// With a fixed source of rolls, for tests.
    pub fn with_roll(roll: impl Fn() -> f64 + Send + Sync + 'static) -> Self {
        Self {
            last: Mutex::new(None),
            roll: Box::new(roll),
        }
    }

    /// Holds the next reply back for the rest of a gap. Immediately before
    /// publishing, so the model's own seconds do not eat into it.
    async fn pace(&self, min: u64, max: u64) {
        if min == 0 {
            return;
        }
        let gap = Duration::from_millis(gap_ms(min, max, (self.roll)()));
        let last = *self
            .last
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some(last) = last {
            let waited = last.elapsed();
            if waited < gap {
                let remaining = gap - waited;
                tracing::info!(
                    gap = gap.as_millis() as u64,
                    remaining = remaining.as_millis() as u64,
                    "pacing: holding the next reply"
                );
                tokio::time::sleep(remaining).await;
            }
        }
        *self
            .last
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(tokio::time::Instant::now());
    }
}

/// What became of one mention.
enum Handled {
    /// Answered (or rehearsed) with this text.
    Said(String),
    /// Set aside or held, not a failure.
    Aside(String),
    Failed(String),
}

/// The journal line's turn: what the draft said and did.
#[derive(Default)]
struct Turn {
    tone: Option<Tone>,
    recalled: Vec<String>,
    tools: Vec<ToolCall>,
    thinking: String,
    text: String,
    declined: Vec<String>,
}

impl Turn {
    fn thinking(thinking: impl Into<String>) -> Self {
        Self {
            thinking: thinking.into(),
            ..Self::default()
        }
    }

    fn of(draft: &ReplyDraft, text: &str) -> Self {
        Self {
            tone: Some(draft.incoming_tone),
            recalled: draft.recalled.clone(),
            tools: draft.tools.clone(),
            thinking: draft.thinking.clone(),
            text: text.to_owned(),
            declined: draft.declined.clone(),
        }
    }
}

/// One line in the journal per mention, whatever happened to it.
async fn journal(
    ctx: &JobCtx,
    post: &XPost,
    incoming: &str,
    turn: Turn,
    outcome: Outcome,
    note: Option<String>,
) {
    let said = if matches!(outcome, Outcome::Posted | Outcome::DryRun) {
        turn.text
    } else {
        String::new()
    };
    let entry = ReflectionInput {
        id: Some(format!("mention:{}", post.id)),
        at: None,
        trigger: Trigger::Mention,
        incoming: incoming.to_owned(),
        tone: turn.tone.unwrap_or(Tone::Neutral),
        recalled: turn.recalled,
        tools: turn.tools,
        thinking: turn.thinking,
        said,
        declined: turn.declined,
        outcome,
        outcome_note: note,
    };
    record_quietly(ctx.journal.as_ref(), entry).await;
}

fn lane_name(lane: ReplyLane) -> &'static str {
    match lane {
        ReplyLane::Auto => "auto",
        ReplyLane::Smart => "smart",
        ReplyLane::Called => "called",
    }
}

/// The batch, regulars first: several batches' worth are read and the
/// people who interact most are taken. Without a people record, oldest first.
async fn take(
    ctx: &JobCtx,
    conversation: &Conversation,
) -> anyhow::Result<Vec<QueuedMention<QueuedXPost>>> {
    let max = ctx.config.reply.max_per_run as usize;
    let Some(people) = &conversation.people else {
        return Ok(conversation.queue.take_pending(max).await?);
    };
    let mut pending = conversation.queue.take_pending(max * 5).await?;
    if pending.len() <= max {
        return Ok(pending);
    }
    let authors: Vec<String> = pending
        .iter()
        .filter_map(|item| item.post.0.author_id.as_ref().map(ToString::to_string))
        .collect();
    let scores = people.scores(&authors).await.unwrap_or_default();
    let score = |item: &QueuedMention<QueuedXPost>| {
        item.post
            .0
            .author_id
            .as_ref()
            .and_then(|id| scores.get(id.as_str()).copied())
            .unwrap_or(0)
    };
    pending.sort_by(|a, b| {
        score(b)
            .cmp(&score(a))
            .then_with(|| a.cached_at.cmp(&b.cached_at))
    });
    pending.truncate(max);
    Ok(pending)
}

/// A like, once, never in a dry run, never in the way of the reply.
async fn like(ctx: &JobCtx, conversation: &Conversation, item: &QueuedMention<QueuedXPost>) {
    if !ctx.config.reply.like_mentions || item.liked_at.is_some() || ctx.config.dry_run {
        return;
    }
    let post = &item.post.0;
    let liked = async {
        conversation.outlet.like(&post.id).await?;
        conversation.queue.mark_liked(post.id.as_str()).await?;
        anyhow::Ok(())
    };
    if let Err(error) = liked.await {
        tracing::warn!(%error, post_id = %post.id, "could not like the mention");
    }
}

/// A file the drafter generated, uploaded. Fails soft: the reply was written
/// to stand on its own.
async fn attach_generated(
    ctx: &JobCtx,
    conversation: &Conversation,
    media_id: &str,
) -> Vec<String> {
    let Some(stored) = ctx
        .deps
        .media
        .as_ref()
        .and_then(|media| media.store.get(media_id))
    else {
        tracing::warn!(
            media_id,
            "the drafted mediaId is not in the store; replying with text only"
        );
        return Vec::new();
    };
    let media = Media {
        data: stored.data,
        mime: x_media_type(stored.mime_type),
    };
    match conversation.outlet.upload(media).await {
        Ok(uploaded) => vec![uploaded],
        Err(error) => {
            tracing::warn!(%error, media_id, "generated media upload failed; replying with text only");
            Vec::new()
        }
    }
}

fn x_media_type(mime: teeny_media::types::MimeType) -> MediaType {
    use teeny_media::types::MimeType;
    match mime {
        MimeType::Png => MediaType::Png,
        MimeType::Jpeg => MediaType::Jpeg,
        MimeType::Webp => MediaType::Webp,
        MimeType::Gif => MediaType::Gif,
        MimeType::Mp4 => MediaType::Mp4,
    }
}

/// The drafter's GIF query as an uploaded media id: the account's own
/// channel first, then the rest of Giphy. Fails soft.
async fn attach_gif(ctx: &JobCtx, conversation: &Conversation, query: &str) -> Vec<String> {
    let Some(gifs) = &ctx.deps.gifs else {
        return Vec::new();
    };
    let attached = async {
        let own = gifs.search(query, true).await?.into_iter().next();
        let from_channel = own.is_some();
        let gif = match own {
            Some(gif) => Some(gif),
            None => gifs.search(query, false).await?.into_iter().next(),
        };
        let Some(gif) = gif else {
            tracing::info!(query, "no GIF matched the drafted query");
            return anyhow::Ok(Vec::new());
        };
        tracing::info!(query, gif_id = gif.id, from_channel, "attaching a GIF");
        let (data, mime) = gifs.download(&gif).await?;
        let media = Media {
            data,
            mime: x_media_type(mime),
        };
        Ok(vec![conversation.outlet.upload(media).await?])
    };
    attached.await.unwrap_or_else(|error| {
        tracing::warn!(%error, query, "GIF attachment failed; replying with text only");
        Vec::new()
    })
}

/// Everything after the self-reply guard; an error marks the mention failed.
async fn answer(
    ctx: &JobCtx,
    conversation: &Conversation,
    item: &QueuedMention<QueuedXPost>,
    bot_user_id: &UserId,
) -> anyhow::Result<Handled> {
    let post = &item.post.0;
    let id = post.id.as_str();
    let dry_run = ctx.config.dry_run;

    // A like first, for everyone: it costs nothing, and the person sees
    // something back even when the answer never comes.
    like(ctx, conversation, item).await;

    // The budget, before the model is paid for a draft that would not go out.
    let assessed: Option<Assessment> = match &ctx.deps.reply_budget {
        Some(budget) => Some(
            budget
                .assess(
                    &Mention {
                        text: post.text.clone(),
                        conversation_id: post.conversation_id.as_ref().map(ToString::to_string),
                        in_reply_to_user_id: post
                            .in_reply_to_user_id
                            .as_ref()
                            .map(ToString::to_string),
                    },
                    bot_user_id.as_str(),
                )
                .await?,
        ),
        None => None,
    };
    if let Some(assessed) = assessed.as_ref().filter(|assessed| !assessed.allowed) {
        let lane = lane_name(assessed.lane);
        let cap = assessed.cap.map(|cap| cap.to_string()).unwrap_or_default();
        conversation
            .queue
            .mark_skipped(
                id,
                &format!("budget: {lane} lane at {}/{cap}", assessed.used),
            )
            .await?;
        tracing::info!(post_id = id, lane, own_post_id = ?assessed.own_post_id, used = assessed.used, cap, "set a mention aside: the reply budget for its lane is spent");
        journal(
            ctx,
            post,
            &post.text,
            Turn::thinking(format!(
                "The {lane} reply budget is spent ({} of {cap}), so this one gets a like and no answer.",
                assessed.used
            )),
            Outcome::Skipped,
            Some(format!("reply budget: {lane} lane spent")),
        )
        .await;
        return Ok(Handled::Aside(format!("budget: {lane} lane spent")));
    }

    // The gate, inbound. An operator's approval waives it; the outbound
    // check is never waived: the approval was of the question, not the answer.
    let context = HoldContext {
        post_id: Some(id.to_owned()),
        conversation_id: post.conversation_id.as_ref().map(ToString::to_string),
        author_username: post.author_username.clone(),
        ..HoldContext::new(HoldSurface::X)
    };
    if item.filter_approved != Some(true)
        && let SafetyDecision::Held(held) =
            ctx.deps.safety.inbound(&post.text, Some(&context)).await
    {
        conversation
            .queue
            .mark_failed(id, &format!("held: {}", held.category))
            .await?;
        tracing::info!(post_id = id, category = held.category, by = ?held.by, reason = held.reason, "held a mention: the content filter refused it");
        journal(
            ctx,
            post,
            &format!("[held by the content filter: {}]", held.category),
            Turn::thinking("This is not something I talk about, so I am not answering it."),
            Outcome::Skipped,
            Some(format!("content filter: {}", held.category)),
        )
        .await;
        return Ok(Handled::Aside(format!("held: {}", held.category)));
    }

    let drafted = generate(ctx, post).await?;
    let styled = apply_house_style(
        &drafted.text,
        &HouseStyleOptions {
            max_emoji: ctx.config.agent.max_emoji_per_post as usize,
            replying_to: post.author_username.clone(),
            ..HouseStyleOptions::default()
        },
    );
    if !styled.fixes.is_empty() {
        tracing::info!(post_id = id, fixes = ?styled.fixes, "house style corrected the draft");
    }

    // The gate, outbound: what would be published is what is checked, and
    // what comes back (links cut) is what goes out.
    let outgoing = ctx.deps.safety.outbound(&styled.text, Some(&context)).await;
    if !outgoing.fixes.is_empty() {
        tracing::warn!(post_id = id, fixes = ?outgoing.fixes, "the content filter edited the draft");
    }
    if let SafetyDecision::Held(held) = &outgoing.decision {
        conversation
            .queue
            .mark_failed(id, &format!("withheld: {}", held.category))
            .await?;
        tracing::warn!(post_id = id, category = held.category, by = ?held.by, reason = held.reason, "withheld a reply: the content filter refused the draft");
        journal(
            ctx,
            post,
            &post.text,
            Turn::of(&drafted, ""),
            Outcome::Held,
            Some(format!(
                "content filter withheld the draft: {}",
                held.category
            )),
        )
        .await;
        return Ok(Handled::Aside(format!("withheld: {}", held.category)));
    }
    let text = outgoing.text;

    let media_ids = match (&drafted.media_id, &drafted.gif_query) {
        _ if dry_run => Vec::new(),
        (Some(media_id), _) => attach_generated(ctx, conversation, media_id).await,
        (None, Some(query)) => attach_gif(ctx, conversation, query).await,
        (None, None) => Vec::new(),
    };
    let posted = if dry_run {
        None
    } else {
        conversation
            .pacing
            .pace(
                ctx.config.reply.min_interval_ms,
                ctx.config.reply.max_interval_ms,
            )
            .await;
        Some(
            conversation
                .outlet
                .reply(&text, &post.id, media_ids.clone())
                .await?,
        )
    };
    let posted_id = posted.as_ref().map(ToString::to_string);

    // Memory keeps what was said, and a rehearsal said nothing.
    if let Some(posted_id) = posted_id.as_deref() {
        remember(ctx, post, &text, Some(posted_id), drafted.incoming_tone).await;
    }
    let sour = is_sour(drafted.incoming_tone);
    if sour {
        tracing::info!(post_id = id, tone = ?drafted.incoming_tone, "answered but not remembered: the incoming post read as sour");
    }
    conversation
        .queue
        .mark_replied(
            id,
            &Reply {
                id: posted_id.clone(),
                text: text.clone(),
                lane: assessed.as_ref().map(|assessed| assessed.lane),
                own_post_id: assessed
                    .as_ref()
                    .and_then(|assessed| assessed.own_post_id.clone()),
            },
        )
        .await?;
    if let (Some(people), Some(author)) = (&conversation.people, &post.author_id) {
        let observed = people
            .observe(&Observation {
                user_id: author.to_string(),
                username: post.author_username.clone(),
                kind: Interaction::Answered,
                at: None,
            })
            .await;
        if let Err(error) = observed {
            tracing::debug!(%error, "could not record who was answered");
        }
    }
    journal(
        ctx,
        post,
        &post.text,
        Turn::of(&drafted, &text),
        if dry_run {
            Outcome::DryRun
        } else {
            Outcome::Posted
        },
        sour.then(|| "sour tone: answered, not remembered".to_owned()),
    )
    .await;
    tracing::info!(post_id = id, reply_id = ?posted_id, text, gif_query = ?drafted.gif_query, media_id = ?drafted.media_id, media_attached = !media_ids.is_empty(), dry_run, "replied to a cached mention");
    Ok(Handled::Said(text))
}

/// One mention, whatever happens to it.
async fn handle(
    ctx: &JobCtx,
    conversation: &Conversation,
    item: &QueuedMention<QueuedXPost>,
    bot_user_id: &UserId,
) -> Handled {
    let post = &item.post.0;
    // Every path upstream already excludes the account's own posts; this is
    // the last one standing. A self-reply is the failure that compounds.
    if post.author_id.as_ref() == Some(bot_user_id) {
        if let Err(error) = conversation
            .queue
            .mark_failed(post.id.as_str(), "refused: the account wrote this post")
            .await
        {
            tracing::warn!(%error, post_id = %post.id, "could not mark the account's own post");
        }
        tracing::warn!(post_id = %post.id, "refused to answer the account's own post");
        journal(
            ctx,
            post,
            &post.text,
            Turn::thinking(
                "This is my own post. Answering it would start a loop of me replying to myself.",
            ),
            Outcome::Skipped,
            Some("the account wrote this post".into()),
        )
        .await;
        return Handled::Aside("refused: the account wrote this post".into());
    }
    match answer(ctx, conversation, item, bot_user_id).await {
        Ok(handled) => handled,
        Err(error) => {
            let message = error.to_string();
            if let Err(error) = conversation
                .queue
                .mark_failed(post.id.as_str(), &message)
                .await
            {
                tracing::warn!(%error, post_id = %post.id, "could not mark the mention failed");
            }
            tracing::error!(%message, post_id = %post.id, attempts = item.attempts + 1, "failed to reply to a cached mention");
            journal(
                ctx,
                post,
                &post.text,
                Turn::default(),
                Outcome::Failed,
                Some(message.clone()),
            )
            .await;
            Handled::Failed(message)
        }
    }
}

/// One batch. `ran` is whether it took a batch, which tells a worker loop to
/// come straight back rather than idle.
pub async fn run(ctx: &JobCtx) -> anyhow::Result<JobReport> {
    let conversation = ctx.conversation()?;
    let queued = take(ctx, conversation).await?;
    if queued.is_empty() {
        return Ok(JobReport::declined(JOB, "the queue is empty"));
    }
    // Not knowing which posts are the account's own stops the run rather
    // than relaxing the check. The queue keeps until the next tick.
    let bot_user_id = match conversation.account.bot_user_id().await {
        Ok(Some(id)) if !id.as_str().is_empty() => id,
        Ok(_) | Err(_) => {
            tracing::error!("cannot identify the account; refusing to reply to anything this run");
            return Ok(JobReport::declined(JOB, "cannot identify the account"));
        }
    };

    let mut report = JobReport::ran(JOB, Outcome::Skipped);
    let mut notes = Vec::new();
    for item in &queued {
        match handle(ctx, conversation, item, &bot_user_id).await {
            Handled::Said(text) => report.drafts.push(text),
            Handled::Aside(note) => notes.push(note),
            Handled::Failed(problem) => report.problems.push(problem),
        }
    }
    report.outcome = match (report.drafts.is_empty(), report.problems.is_empty()) {
        (false, _) if ctx.config.dry_run => Outcome::DryRun,
        (false, _) => Outcome::Posted,
        (true, false) => Outcome::Failed,
        (true, true) => Outcome::Skipped,
    };
    if !notes.is_empty() {
        report.note = Some(notes.join("; "));
    }
    Ok(report)
}

#[cfg(test)]
#[path = "job_tests.rs"]
mod tests;
