//! The content calendar, run: one item at a time, on its cron. A port of
//! `legacy/src/jobs/calendar-job.ts` and the drafting half of
//! `legacy/src/agent/content-model.ts`.
//!
//! The item says whether it carries a picture, and the publisher agent draws
//! it; an item that promised a picture and did not get one is held, not
//! published bare. The draft passes the house style and the content filter
//! in host code before any channel sees it. X goes through the cadence and
//! the publisher; Telegram through a [`TelegramSender`] the service wires
//! (absent, a Telegram channel is a failure line). The same post twice in an
//! hour is the pattern that once cost the account its reach, so each item
//! keeps an hour's guard.

use std::sync::Arc;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use chrono::TimeDelta;
use serde_json::{Value, json};
use teeny_agent::posture::Belt;
use teeny_cadence::calendar::{Aspect, CalendarItem, Channel, Media};
use teeny_media::store::{MediaStore, StoredMedia, is_media_id};
use teeny_media::types::MediaKind;
use teeny_memory::Memory;
use teeny_safety::gate::{SafetyDecision, SafetyGate};
use teeny_safety::house_style::{HouseStyleOptions, apply_house_style, truncate_at_boundary};
use teeny_store::ReflectionInput;
use teeny_store::holds::{HoldContext, HoldKind, HoldSurface};
use teeny_store::journal::{Outcome, Tone, ToolCall, Trigger};
use teeny_x::MAX_POST_LENGTH;

use crate::ctx::{JobCtx, JobReport};
use crate::journal_glue::{produced_media_id, record_quietly, tool_steps};
use crate::morning::post::{Attachment, OwnPost, OwnPoster, XOwnPoster};
use crate::morning::stages::{Fields, Shape, extract_json};
use crate::runner::{AgentKey, AgentRun, AgentRunner, Origin};

/// Telegram's longest photo caption.
pub const TELEGRAM_MAX_CAPTION_LENGTH: usize = 1_024;

/// How long an item rests after it ran.
pub const ITEM_INTERVAL: TimeDelta = TimeDelta::hours(1);

/// What the drafter hands back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentDraft {
    pub text: String,
    /// A file the drafter generated this run, when the item carries media.
    pub media_id: Option<String>,
    pub thinking: String,
    pub declined: Vec<String>,
    pub tools: Vec<ToolCall>,
}

/// Writes one item's post.
#[async_trait]
pub trait ContentDrafter: Send + Sync {
    async fn draft(&self, item: &CalendarItem, today: &str, memory: &str) -> Result<ContentDraft>;
}

/// Posts to Telegram chats (the OpenHuman channel, or a Bot API client).
#[async_trait]
pub trait TelegramSender: Send + Sync {
    /// The sent message's id.
    async fn send_message(&self, chat_id: i64, text: &str) -> Result<i64>;
    /// A photo or a clip, with a caption when it fits.
    async fn send_media(
        &self,
        chat_id: i64,
        media: &StoredMedia,
        caption: Option<&str>,
    ) -> Result<i64>;
}

/// The answer the drafter is asked for.
struct DraftShape {
    text: String,
    media_id: Option<String>,
    thinking: String,
    declined: Vec<String>,
}

impl Shape for DraftShape {
    fn read(value: &Value) -> Result<Self, Vec<String>> {
        let mut fields = Fields::new(value);
        let shape = Self {
            text: fields.string_or("text", MAX_POST_LENGTH, ""),
            media_id: fields.optional_string("mediaId", usize::MAX),
            thinking: fields.string_or("thinking", 600, ""),
            declined: fields.list("declined", 0, 4, |fields, path, value| {
                fields.item_string(path, value, 0, 120)
            }),
        };
        fields.finish(shape)
    }
}

/// The publisher agent as the drafter: one trusted scheduled turn, told to
/// return the draft rather than post it.
pub struct RunnerDrafter {
    pub runner: Arc<dyn AgentRunner>,
}

fn wants(item: &CalendarItem) -> String {
    match item.media {
        Media::None => {
            "This post is text only; there is no generation tool and nothing to attach.".into()
        }
        media => {
            let (thing, tool) = match media {
                Media::Video => ("a short video", "video"),
                _ => ("an image", "image"),
            };
            let aspect = match item.aspect {
                Aspect::Square => "square",
                Aspect::Landscape => "landscape",
                Aspect::Portrait => "portrait",
            };
            format!(
                "This post CARRIES {thing} ({aspect}{}). Generate it once with generate_{tool} and return its mediaId.",
                if item.features_tiny {
                    ", featuring Tiny"
                } else {
                    ", with no Tiny in it"
                }
            )
        }
    }
}

fn channels(item: &CalendarItem) -> &'static str {
    let x = item.channels.contains(&Channel::X);
    let telegram = item.channels.contains(&Channel::Telegram);
    match (x, telegram) {
        (true, true) => "the X timeline and the Telegram group",
        (true, false) => "the X timeline",
        _ => "the Telegram group",
    }
}

/// The task, as the legacy content agent was given it, with the shape of
/// the answer and the rule that the host posts, not the agent.
pub fn draft_task(item: &CalendarItem, today: &str, memory: &str) -> String {
    [
        format!("Write today's \"{}\" post. Today is {today}.", item.name),
        String::new(),
        "THE BRIEF (from the operator; follow it, do not quote it):".into(),
        item.brief.clone(),
        String::new(),
        wants(item),
        String::new(),
        "WHAT THIS ITEM HAS POSTED ON EARLIER DAYS (untrusted, from memory; do not repeat any of it):".into(),
        if memory.is_empty() {
            "(nothing recalled: this is the first one, and do not pretend otherwise)".into()
        } else {
            memory.to_owned()
        },
        String::new(),
        format!(
            "The post goes to {}. Do not publish it yourself and do not call x_post: the service posts it after its own checks.",
            channels(item)
        ),
        "Answer with one JSON object and nothing else: {\"text\": string (the post, plain text, under 280 characters; empty only when the brief allows the picture alone), \"mediaId\"?: string (the id a generate tool returned this run), \"thinking\": string (two to four sentences, first person: what memory said about earlier days, what you chose and why), \"declined\": [string] (at most four things you chose not to say or draw, with the reason)}".into(),
    ]
    .join("\n")
}

#[async_trait]
impl ContentDrafter for RunnerDrafter {
    async fn draft(&self, item: &CalendarItem, today: &str, memory: &str) -> Result<ContentDraft> {
        let outcome = self
            .runner
            .run(AgentRun {
                agent: AgentKey::Belt(Belt::Publisher),
                origin: Origin::Scheduled { once: true },
                prompt: draft_task(item, today, memory),
                session: None,
                model: None,
                label: format!("calendar.{}", item.name),
            })
            .await?;
        if outcome
            .tool_calls
            .iter()
            .any(|call| call.name == "x_post" && !call.is_error)
        {
            return Err(anyhow!(
                "the drafter called x_post itself; nothing more was published"
            ));
        }
        let shape = extract_json(&outcome.reply)
            .and_then(|value| DraftShape::read(&value).ok())
            .ok_or_else(|| anyhow!("The content agent returned no usable draft"))?;
        let media_id = shape
            .media_id
            .map(|id| id.trim().to_owned())
            .filter(|id| is_media_id(id) && produced_media_id(&outcome.tool_calls, id));
        Ok(ContentDraft {
            text: truncate_at_boundary(shape.text.trim(), MAX_POST_LENGTH),
            media_id,
            thinking: shape.thinking.trim().to_owned(),
            declined: shape
                .declined
                .iter()
                .map(|line| line.trim().to_owned())
                .filter(|line| !line.is_empty())
                .collect(),
            tools: tool_steps(&outcome.tool_calls),
        })
    }
}

/// Everything one calendar run reaches.
pub struct CalendarJob {
    pub drafter: Arc<dyn ContentDrafter>,
    pub memory: Arc<dyn Memory>,
    pub poster: Arc<dyn OwnPoster>,
    pub telegram: Option<Arc<dyn TelegramSender>>,
    /// Where the drafter's generated file is.
    pub media: Option<Arc<MediaStore>>,
    pub safety: Option<Arc<SafetyGate>>,
    /// `CALENDAR_TELEGRAM_CHAT_IDS`: chats an item names none of its own.
    pub telegram_chat_ids: Vec<String>,
    pub max_emoji: usize,
    /// `DRY_RUN`.
    pub dry_run: bool,
}

impl CalendarJob {
    /// From the clients: the publisher agent drafts, X through the cadence
    /// and the publisher, no Telegram until [`with_telegram`](Self::with_telegram).
    pub fn from_ctx(ctx: &JobCtx) -> Self {
        let deps = &ctx.deps;
        Self {
            drafter: Arc::new(RunnerDrafter {
                runner: Arc::clone(&ctx.runner),
            }),
            memory: Arc::clone(&deps.memory),
            poster: Arc::new(XOwnPoster::from_deps(
                deps,
                HoldSurface::Calendar,
                "calendar",
            )),
            telegram: None,
            media: deps.media.as_ref().map(|media| Arc::clone(&media.store)),
            safety: Some(Arc::clone(&deps.safety)),
            telegram_chat_ids: ctx.config.calendar.telegram_chat_ids.clone(),
            max_emoji: deps.settings.max_emoji,
            dry_run: deps.settings.dry_run,
        }
    }

    pub fn with_telegram(mut self, telegram: Arc<dyn TelegramSender>) -> Self {
        self.telegram = Some(telegram);
        self
    }

    /// Runs one item: the hour's guard, the cadence (X only), the draft,
    /// the checks, the channels, the memory, the journal. A forced run
    /// skips the guard and the cadence.
    pub async fn run(&self, ctx: &JobCtx, item: &CalendarItem, force: bool) -> Result<JobReport> {
        let job = format!("calendar:{}", item.name);
        let now = ctx.now();
        let guard = ctx.guard_with(&job, ITEM_INTERVAL);
        if !force {
            if guard.check(now).is_err() {
                return Ok(JobReport::declined(&job, "calendar: skipping, it just ran"));
            }
            if item.channels.contains(&Channel::X)
                && let Ok(claim) = ctx.deps.cadence.claim(&job).await
                && !claim.is_granted()
            {
                return Ok(JobReport::declined(
                    &job,
                    format!("calendar: skipping, the cadence says {claim:?}"),
                ));
            }
        }
        guard.mark(now);

        let today = now.format("%Y-%m-%d").to_string();
        let id = format!("calendar:{}:{today}", item.name);
        let memory = self
            .memory
            .recall(
                &format!(
                    "What has this account posted for its scheduled \"{}\" calendar item on earlier days? For each: the text, and the scene of the picture if it had one.",
                    item.name
                ),
                None,
            )
            .await
            .unwrap_or_default();

        let draft = match self.drafter.draft(item, &today, &memory).await {
            Ok(draft) => draft,
            Err(error) => {
                let note = error.to_string();
                self.journal(ctx, &id, item, None, Outcome::Failed, Some(&note))
                    .await;
                return Ok(finished(&job, Outcome::Failed, Some(note), None));
            }
        };
        let mut text = apply_house_style(
            &draft.text,
            &HouseStyleOptions {
                max_emoji: self.max_emoji,
                max_length: MAX_POST_LENGTH,
                replying_to: None,
            },
        )
        .text;
        let generated = draft
            .media_id
            .as_deref()
            .and_then(|media_id| self.media.as_ref()?.get(media_id));
        let styled = ContentDraft {
            text: text.clone(),
            ..draft.clone()
        };
        if item.media != Media::None && generated.is_none() {
            let note = match &draft.media_id {
                Some(media_id) => format!("the drafted mediaId {media_id} is not in the store"),
                None => format!(
                    "the item carries {} and none was generated",
                    if item.media == Media::Image {
                        "an image"
                    } else {
                        "a video"
                    }
                ),
            };
            self.journal(ctx, &id, item, Some(&styled), Outcome::Held, Some(&note))
                .await;
            return Ok(finished(&job, Outcome::Held, Some(note), None));
        }
        if text.is_empty() && generated.is_none() {
            let note = "the draft was empty";
            self.journal(ctx, &id, item, Some(&styled), Outcome::Held, Some(note))
                .await;
            return Ok(finished(&job, Outcome::Held, Some(note.into()), None));
        }

        // The gate, on what would go out on every channel.
        if let Some(safety) = &self.safety {
            let outgoing = safety
                .outbound(
                    &text,
                    Some(&HoldContext {
                        kind: Some(HoldKind::Draft),
                        tool: Some(item.name.clone()),
                        ..HoldContext::new(HoldSurface::Calendar)
                    }),
                )
                .await;
            if !outgoing.fixes.is_empty() {
                tracing::warn!(item = item.name, fixes = ?outgoing.fixes, "calendar: the content filter edited the draft");
            }
            match outgoing.decision {
                SafetyDecision::Clean => text = outgoing.text,
                SafetyDecision::Held(hold) => {
                    let note = format!("content filter withheld the draft: {}", hold.category);
                    let empty = ContentDraft {
                        text: String::new(),
                        ..draft.clone()
                    };
                    self.journal(ctx, &id, item, Some(&empty), Outcome::Held, Some(&note))
                        .await;
                    return Ok(finished(&job, Outcome::Held, Some(note), None));
                }
            }
        }
        let checked = ContentDraft {
            text: text.clone(),
            ..draft
        };

        if self.dry_run {
            tracing::info!(
                item = item.name,
                text,
                "DRY RUN: would have published a calendar post"
            );
            self.remember(&id, item, &text, generated.as_ref(), false, &[])
                .await;
            self.journal(ctx, &id, item, Some(&checked), Outcome::DryRun, None)
                .await;
            return Ok(finished(&job, Outcome::DryRun, None, Some(text)));
        }

        let mut results: Vec<String> = Vec::new();
        let mut failures: Vec<String> = Vec::new();
        if item.channels.contains(&Channel::X) {
            let post = OwnPost {
                kind: job.clone(),
                text: if text.is_empty() {
                    " ".into()
                } else {
                    text.clone()
                },
                media: generated.as_ref().map(|media| Attachment {
                    data: media.data.clone(),
                    mime_type: media.mime_type,
                }),
                quote: None,
            };
            match self.poster.post(post).await {
                Ok(posted) => results.push(format!("x:{}", posted.id)),
                Err(error) => failures.push(format!("x: {error}")),
            }
        }
        if item.channels.contains(&Channel::Telegram) {
            let chats = item
                .telegram_chat_ids
                .clone()
                .unwrap_or_else(|| self.telegram_chat_ids.clone());
            match &self.telegram {
                Some(telegram) if !chats.is_empty() => {
                    for chat in chats {
                        match self
                            .telegram_send(telegram.as_ref(), &chat, &text, generated.as_ref())
                            .await
                        {
                            Ok(sent) => results.push(format!("telegram:{chat}:{sent}")),
                            Err(error) => failures.push(format!("telegram {chat}: {error}")),
                        }
                    }
                }
                _ => failures.push("telegram: no client or no chat ids configured".into()),
            }
        }

        let outcome = if results.is_empty() {
            Outcome::Failed
        } else {
            Outcome::Posted
        };
        tracing::info!(item = item.name, ?results, ?failures, "calendar: done");
        if !results.is_empty() {
            self.remember(&id, item, &text, generated.as_ref(), true, &results)
                .await;
        }
        let note = (!failures.is_empty()).then(|| failures.join("; "));
        self.journal(ctx, &id, item, Some(&checked), outcome, note.as_deref())
            .await;
        let mut report = finished(&job, outcome, note, Some(text));
        report.posted_id = results
            .iter()
            .find_map(|result| result.strip_prefix("x:"))
            .map(str::to_owned);
        Ok(report)
    }

    async fn telegram_send(
        &self,
        telegram: &dyn TelegramSender,
        chat: &str,
        text: &str,
        generated: Option<&StoredMedia>,
    ) -> Result<i64> {
        let chat_id: i64 = chat.parse()?;
        let caption = (text.encode_utf16().count() <= TELEGRAM_MAX_CAPTION_LENGTH
            && !text.is_empty())
        .then_some(text);
        match generated {
            Some(media) => {
                let sent = telegram.send_media(chat_id, media, caption).await?;
                if caption.is_none() && !text.is_empty() {
                    telegram.send_message(chat_id, text).await?;
                }
                Ok(sent)
            }
            None => telegram.send_message(chat_id, text).await,
        }
    }

    /// Written back deterministically, so tomorrow's run can see today's scene.
    async fn remember(
        &self,
        id: &str,
        item: &CalendarItem,
        text: &str,
        media: Option<&StoredMedia>,
        published: bool,
        results: &[String],
    ) {
        let channels: Vec<&str> = item
            .channels
            .iter()
            .map(|channel| match channel {
                Channel::X => "x",
                Channel::Telegram => "telegram",
            })
            .collect();
        let mut payload = json!({
            "item": item.name,
            "text": text,
            "channels": channels,
            "published": published,
            "results": results,
        });
        if let Some(media) = media {
            payload["media"] = json!(match media.kind {
                MediaKind::Image => "image",
                MediaKind::Video => "video",
            });
            payload["mediaId"] = json!(media.id);
            payload["prompt"] = json!(media.prompt);
        }
        if let Err(error) = self
            .memory
            .capture_activity("calendar_post", id, payload, None)
            .await
        {
            tracing::warn!(%error, item = item.name, "calendar: could not remember the post");
        }
    }

    async fn journal(
        &self,
        ctx: &JobCtx,
        id: &str,
        item: &CalendarItem,
        draft: Option<&ContentDraft>,
        outcome: Outcome,
        note: Option<&str>,
    ) {
        record_quietly(
            ctx.journal.as_ref(),
            ReflectionInput {
                id: Some(id.to_owned()),
                at: Some(teeny_store::iso(ctx.now())),
                trigger: Trigger::Calendar,
                incoming: format!("{}: {}", item.name, item.brief),
                tone: Tone::Neutral,
                recalled: Vec::new(),
                tools: draft.map(|draft| draft.tools.clone()).unwrap_or_default(),
                thinking: draft
                    .map(|draft| draft.thinking.clone())
                    .unwrap_or_default(),
                said: match outcome {
                    Outcome::Posted | Outcome::DryRun => {
                        draft.map(|draft| draft.text.clone()).unwrap_or_default()
                    }
                    _ => String::new(),
                },
                declined: draft
                    .map(|draft| draft.declined.clone())
                    .unwrap_or_default(),
                outcome,
                outcome_note: note.map(str::to_owned),
            },
        )
        .await;
    }
}

fn finished(job: &str, outcome: Outcome, note: Option<String>, text: Option<String>) -> JobReport {
    let mut report = JobReport::ran(job, outcome);
    report.note = note;
    report.drafts = text.filter(|text| !text.is_empty()).into_iter().collect();
    report
}

/// The scheduled run of one item (no Telegram: see
/// [`CalendarJob::with_telegram`]).
pub async fn run(ctx: &JobCtx, item: &CalendarItem) -> Result<JobReport> {
    CalendarJob::from_ctx(ctx).run(ctx, item, false).await
}

/// The rehearsal (`calendar-dry-run`): the item drafted and checked, nothing
/// published, whatever `DRY_RUN` says, no guard.
pub async fn dry_run(ctx: &JobCtx, item: &CalendarItem) -> Result<JobReport> {
    let mut job = CalendarJob::from_ctx(ctx);
    job.dry_run = true;
    job.run(ctx, item, true).await
}

#[cfg(test)]
#[path = "job_tests.rs"]
mod tests;
