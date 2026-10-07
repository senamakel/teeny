//! A Telegram message turned into a chat turn: the turn handling of
//! `legacy/src/telegram/bot.ts` and `legacy/src/harness/chat-harness.ts`,
//! without the transport. Whatever carries the messages (OpenHuman's
//! Telegram channel, or a fallback poller) calls [`keep_line`] for every line
//! it sees and [`handle_message`] for every message addressed to the
//! account, and sends what comes back.
//!
//! The turn is a `teeny-chat` agent with `Origin::Public`: it holds no write
//! tool. Host code checks the draft (house style, the content filter) and
//! hands the text to the transport.

mod draft;

pub use draft::{ChatDraft, MAX_CHAT_REPLY_LENGTH, REACTIONS};

use chrono::TimeDelta;
use teeny_agent::posture::Belt;
use teeny_cadence::clock::{iso, parse_iso};
use teeny_memory::{ChatKind, MemoryChatMessage, Person};
use teeny_safety::gate::SafetyDecision;
use teeny_safety::house_style::{HouseStyleOptions, apply_house_style};
use teeny_safety::screen::Direction;
use teeny_store::holds::{HoldContext, HoldSurface};
use teeny_store::transcript::Speaker;
use teeny_store::{ChatTurn, TranscriptEntry};

use crate::ctx::{Conversation, JobCtx};
use crate::reply::harness::is_sour;
use crate::runner::{AgentKey, AgentRun, Origin};

/// The channel's name in memory and the transcript.
pub const CHANNEL: &str = "telegram";

/// The second recall, as on X: what the account has been saying itself.
const OWN_ACTIVITY: &str = "What has this account itself posted recently? Include its daily chart posts, the \
market reads it published, and anything it has said publicly in the last few days.";

/// One message to the account, as the transport hands it over. Several
/// messages typed in a row by one person arrive joined, one per line, under
/// the last one's id, with every id in `batch_ids`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    pub id: String,
    pub chat_id: String,
    pub chat_kind: ChatKind,
    /// A group's title.
    pub chat_title: Option<String>,
    /// The text, with any picture already written in as `[image: …]`.
    pub text: String,
    pub author_id: String,
    pub author_username: Option<String>,
    /// The display name.
    pub author_name: Option<String>,
    /// ISO; when it was sent.
    pub created_at: Option<String>,
    /// A forum topic.
    pub message_thread_id: Option<String>,
    /// Every message this one joins; empty for a single message.
    pub batch_ids: Vec<String>,
}

impl ChatMessage {
    fn ids(&self) -> Vec<String> {
        if self.batch_ids.is_empty() {
            vec![self.id.clone()]
        } else {
            self.batch_ids.clone()
        }
    }

    fn for_memory(&self) -> MemoryChatMessage {
        MemoryChatMessage {
            id: self.id.clone(),
            channel: CHANNEL.into(),
            chat_id: self.chat_id.clone(),
            chat_kind: self.chat_kind,
            text: self.text.clone(),
            author_id: self.author_id.clone(),
            author_username: self.author_username.clone(),
            created_at: self.created_at.clone(),
        }
    }
}

/// Keeps a line the account saw, answered or not: a line in a group that
/// was not for the account still tells the next reply what the room was
/// talking about.
pub async fn keep_line(ctx: &JobCtx, message: &ChatMessage) -> anyhow::Result<()> {
    let conversation = ctx.conversation()?;
    conversation
        .transcript
        .append(&TranscriptEntry {
            chat_id: message.chat_id.clone(),
            message_id: message.id.clone(),
            at: message.created_at.clone().unwrap_or_else(|| iso(ctx.now())),
            from: Speaker::Person,
            author_name: message.author_name.clone(),
            text: message.text.clone(),
        })
        .await?;
    Ok(())
}

/// The account's own line, so the next turn can see what it said.
async fn keep_own_turn(
    ctx: &JobCtx,
    conversation: &Conversation,
    chat_id: &str,
    message_id: String,
    text: &str,
) {
    let kept = conversation
        .transcript
        .append(&TranscriptEntry {
            chat_id: chat_id.to_owned(),
            message_id,
            at: iso(ctx.now()),
            from: Speaker::Bot,
            author_name: None,
            text: text.to_owned(),
        })
        .await;
    if let Err(error) = kept {
        tracing::warn!(%error, chat_id, "could not keep own turn in the transcript");
    }
}

/// Recall, draft, capture: the chat-shaped twin of the reply harness, with
/// the same order and the same reasons for it.
async fn generate(
    ctx: &JobCtx,
    message: &ChatMessage,
    history: &[ChatTurn],
) -> anyhow::Result<ChatDraft> {
    let person = Person::chat(
        CHANNEL,
        &message.author_id,
        message.author_username.as_deref(),
    );
    let conversation = history
        .iter()
        .map(|turn| {
            let who = match turn.from {
                Speaker::Bot => "you",
                Speaker::Person => turn.author_name.as_deref().unwrap_or("someone"),
            };
            format!("{who}: {}", turn.text)
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut query = format!(
        "What context, past interactions, product facts, and tone are relevant to answering this Telegram message? {}",
        message.text
    );
    if !conversation.is_empty() {
        query.push_str(&format!("\n\nThe conversation it sits in:\n{conversation}"));
    }
    let memory = &ctx.deps.memory;
    let (recalled, own) = tokio::join!(
        memory.recall(&query, Some(&person)),
        memory.recall(OWN_ACTIVITY, None)
    );
    let (recalled, own) = (recalled.unwrap_or_default(), own.unwrap_or_default());
    let context = if own.trim().is_empty() {
        recalled
    } else {
        format!("{recalled}\n\nWHAT YOU YOURSELF HAVE POSTED RECENTLY:\n{own}")
    };

    let agent = &ctx.config.agent;
    let task = draft::render_task(&draft::TaskInput {
        message,
        memory: &context,
        history,
        bot_username: ctx.config.x.bot_username.trim_start_matches('@'),
        trusted: &agent.trusted_handles,
        own_tokens: &agent.own_token_addresses,
        self_names: &agent.self_names,
    });
    let outcome = ctx
        .runner
        .run(AgentRun {
            agent: AgentKey::Belt(Belt::Chat),
            origin: Origin::Public,
            prompt: task,
            session: None,
            model: None,
            label: "chat".into(),
        })
        .await?;
    let drafted = draft::parse_answer(&outcome.reply)?;
    if !is_sour(drafted.incoming_tone)
        && let Err(error) = memory.capture_chat_message(&message.for_memory()).await
    {
        tracing::warn!(%error, chat_id = message.chat_id, "could not capture the chat message");
    }
    Ok(drafted)
}

/// Keeps the exchange unless the message read as sour.
async fn remember(ctx: &JobCtx, message: &ChatMessage, said: &str, draft: &ChatDraft) {
    if is_sour(draft.incoming_tone) {
        return;
    }
    if let Err(error) = ctx
        .deps
        .memory
        .capture_chat_reply(&message.for_memory(), said, None)
        .await
    {
        tracing::warn!(%error, chat_id = message.chat_id, "could not capture a telegram reply");
    }
}

async fn answer(ctx: &JobCtx, message: &ChatMessage) -> anyhow::Result<Option<String>> {
    let conversation = ctx.conversation()?;
    // Kept before anything else, so a failure further down does not lose it.
    keep_line(ctx, message).await?;

    // A restart after an hour down would otherwise greet everybody who spoke
    // in the meantime, an hour late, all at once.
    let max_age = TimeDelta::seconds(i64::from(ctx.config.telegram.max_message_age_s));
    if let Some(sent) = message.created_at.as_deref().and_then(parse_iso) {
        let age = ctx.now() - sent;
        if age > max_age {
            tracing::info!(
                chat_id = message.chat_id,
                age_ms = age.num_milliseconds(),
                "skipping a stale telegram message"
            );
            return Ok(None);
        }
    }

    // The gate, inbound. Silence is the whole answer: no reply, nothing kept.
    let context = HoldContext {
        chat_id: Some(message.chat_id.clone()),
        message_id: Some(message.id.clone()),
        message_thread_id: message.message_thread_id.clone(),
        author_username: message.author_username.clone(),
        author_name: message.author_name.clone(),
        ..HoldContext::new(HoldSurface::Telegram)
    };
    let safety = &ctx.deps.safety;
    if let SafetyDecision::Held(held) = safety.inbound(&message.text, Some(&context)).await {
        tracing::info!(chat_id = message.chat_id, category = held.category, by = ?held.by, reason = held.reason, "held a telegram message: the content filter refused it");
        return Ok(None);
    }

    // The conversation so far, as much as the budget holds. A read that
    // fails costs the reply its context, not the reply.
    let history = match conversation
        .transcript
        .recent(
            &message.chat_id,
            ctx.config.telegram.history_tokens as usize,
            &message.ids(),
        )
        .await
    {
        Ok(turns) => turns
            .into_iter()
            .filter(|turn| safety.screen(&turn.text, Direction::Inbound))
            .collect(),
        Err(error) => {
            tracing::warn!(%error, chat_id = message.chat_id, "could not read the chat transcript");
            Vec::new()
        }
    };

    let draft = generate(ctx, message, &history).await?;
    let Some(text) = &draft.text else {
        if draft.reaction.is_none() {
            tracing::warn!(
                chat_id = message.chat_id,
                "telegram draft had neither text nor a reaction"
            );
        }
        return Ok(None);
    };
    let styled = apply_house_style(
        text,
        &HouseStyleOptions {
            max_emoji: ctx.config.agent.max_emoji_per_post as usize,
            max_length: MAX_CHAT_REPLY_LENGTH,
            replying_to: None,
        },
    );
    if !styled.fixes.is_empty() {
        tracing::info!(fixes = ?styled.fixes, "house style applied to a telegram draft");
    }
    // The gate, outbound: what would be sent is what is checked.
    let outgoing = safety.outbound(&styled.text, Some(&context)).await;
    if !outgoing.fixes.is_empty() {
        tracing::warn!(chat_id = message.chat_id, fixes = ?outgoing.fixes, "the content filter edited a telegram draft");
    }
    if let SafetyDecision::Held(held) = &outgoing.decision {
        tracing::warn!(chat_id = message.chat_id, category = held.category, by = ?held.by, reason = held.reason, "withheld a telegram reply: the content filter refused the draft");
        return Ok(None);
    }
    let said = outgoing.text;

    if ctx.config.telegram.dry_run {
        tracing::info!(chat_id = message.chat_id, text = said, reaction = ?draft.reaction, "DRY RUN: would have answered on telegram");
        keep_own_turn(
            ctx,
            conversation,
            &message.chat_id,
            format!("dry-run:{}", message.id),
            &said,
        )
        .await;
        remember(ctx, message, &said, &draft).await;
        return Ok(None);
    }
    keep_own_turn(
        ctx,
        conversation,
        &message.chat_id,
        format!("reply:{}", message.id),
        &said,
    )
    .await;
    remember(ctx, message, &said, &draft).await;
    tracing::info!(chat_id = message.chat_id, kind = message.chat_kind.as_str(), answered = ?message.ids(), tone = ?draft.incoming_tone, "answered on telegram");
    Ok(Some(said))
}

/// One message addressed to the account, answered as one turn: the text for
/// the transport to send, or `None` when the account stays quiet (held,
/// stale, a reaction alone, a dry run, a failed turn).
pub async fn handle_message(ctx: &JobCtx, message: &ChatMessage) -> Option<String> {
    match answer(ctx, message).await {
        Ok(text) => text,
        Err(error) => {
            tracing::error!(%error, chat_id = message.chat_id, message_ids = ?message.ids(), "telegram message failed");
            None
        }
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
