//! What approving a withheld draft on `/safety` does: publishes it as
//! written, on the surface it was written for. A port of `release` in
//! `legacy/src/index.ts`. It honours `DRY_RUN` and `TELEGRAM_DRY_RUN` like
//! every other publish, and an own post goes through the cadence: one the
//! cadence refuses is recorded, not published, and can be released again
//! after the gap. What cannot be published after the fact (a post whose
//! picture is gone, a slot that runs again on its own) is only recorded.

use std::sync::Arc;

use async_trait::async_trait;
use teeny_cadence::cadence::{Cadence, PublishError};
use teeny_core::types::PostId;
use teeny_safety::review::{Release, ReviewError};
use teeny_store::Hold;
use teeny_store::holds::HoldSurface;
use teeny_x::{Draft, NoopGuard, XApi, XPublisher};

/// Sends a text message into a Telegram chat; the sent message's id.
#[async_trait]
pub trait ChatSend: Send + Sync {
    async fn send_reply(
        &self,
        chat_id: i64,
        text: &str,
        reply_to: Option<i64>,
        thread: Option<i64>,
    ) -> anyhow::Result<i64>;
}

/// The cadence's name for a released own post.
const KIND: &str = "release";

/// The release hook.
pub struct HoldRelease {
    pub x: Arc<XApi>,
    pub cadence: Arc<Cadence>,
    /// `DRY_RUN`.
    pub dry_run: bool,
    /// Absent when this process runs no Telegram client.
    pub telegram: Option<Arc<dyn ChatSend>>,
    /// `TELEGRAM_DRY_RUN`.
    pub telegram_dry_run: bool,
}

fn effect(error: impl std::fmt::Display) -> ReviewError {
    ReviewError::Effect(error.to_string())
}

fn number(value: Option<&str>) -> Result<Option<i64>, ReviewError> {
    value
        .map(|value| {
            value
                .parse::<i64>()
                .map_err(|_| effect(format!("not a Telegram id: {value}")))
        })
        .transpose()
}

impl HoldRelease {
    async fn on_x(&self, hold: &Hold) -> Result<String, ReviewError> {
        if self.dry_run {
            return Ok("dry run: would have published on X".into());
        }
        // The operator has read it: the guard has nothing left to say, and
        // the publisher still holds the length.
        let publisher = XPublisher::new(Arc::clone(&self.x), Arc::new(NoopGuard), false);
        if let Some(parent) = &hold.context.post_id {
            let draft = Draft {
                reply_to: Some(PostId::from(parent.as_str())),
                ..Draft::new(hold.text.clone())
            };
            let posted = publisher.post(draft).await.map_err(effect)?;
            return Ok(format!("published on X as {}", posted.id));
        }
        let posted = self
            .cadence
            .publish_own(KIND, Some(&hold.text), || async {
                publisher
                    .post(Draft::new(hold.text.clone()))
                    .await
                    .map(|posted| posted.id)
            })
            .await;
        match posted {
            Ok(id) => Ok(format!("published on X as {id}")),
            Err(PublishError::Held(held)) => Ok(format!("recorded, not published: {held}")),
            Err(error) => Err(effect(error)),
        }
    }

    async fn on_telegram(&self, hold: &Hold, chat_id: &str) -> Result<String, ReviewError> {
        let Some(telegram) = &self.telegram else {
            return Ok("recorded; this process has no Telegram client to send it with".into());
        };
        if self.telegram_dry_run {
            return Ok("dry run: would have sent on Telegram".into());
        }
        let context = &hold.context;
        let chat = number(Some(chat_id))?.unwrap_or_default();
        let sent = telegram
            .send_reply(
                chat,
                &hold.text,
                number(context.message_id.as_deref())?,
                number(context.message_thread_id.as_deref())?,
            )
            .await
            .map_err(effect)?;
        Ok(format!("sent on Telegram as {sent}"))
    }
}

#[async_trait]
impl Release for HoldRelease {
    async fn release(&self, hold: &Hold) -> Result<String, ReviewError> {
        let context = &hold.context;
        Ok(match context.surface {
            HoldSurface::X | HoldSurface::Chart => return self.on_x(hold).await,
            HoldSurface::Telegram if context.chat_id.is_some() => {
                let chat = context.chat_id.as_deref().unwrap_or_default();
                return self.on_telegram(hold, chat).await;
            }
            HoldSurface::Calendar => format!(
                "recorded; the \"{}\" item posts again on its next run",
                context.tool.as_deref().unwrap_or("calendar")
            ),
            HoldSurface::Morning => "recorded; the morning post runs again tomorrow".into(),
            HoldSurface::Narrative => {
                "recorded; the serial writes its next episode on schedule".into()
            }
            HoldSurface::Article => "recorded; the next article is written on schedule".into(),
            HoldSurface::Moon => "recorded; the moon chart runs again tomorrow".into(),
            HoldSurface::Quote => "recorded; the quote post runs again tomorrow".into(),
            HoldSurface::Lore => "recorded; the lore post runs again on its next day".into(),
            HoldSurface::Telegram | HoldSurface::Task => {
                "recorded; nothing here can publish it".into()
            }
        })
    }
}

#[cfg(test)]
#[path = "release_tests.rs"]
mod tests;
