//! Where host code posts a drafted reply.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use teeny_core::types::PostId;
use teeny_x::model::Media;
use teeny_x::{Draft, NoopGuard, XApi, XPublisher};

/// The X calls the reply job makes for a public turn, after the turn: the
/// drafter never holds them.
#[async_trait]
pub trait ReplyOutlet: Send + Sync {
    /// Uploads one attachment; its media id.
    async fn upload(&self, media: Media) -> anyhow::Result<String>;
    /// Publishes `text` under `reply_to`; the new post's id.
    async fn reply(
        &self,
        text: &str,
        reply_to: &PostId,
        media_ids: Vec<String>,
    ) -> anyhow::Result<PostId>;
    async fn like(&self, post_id: &PostId) -> anyhow::Result<()>;
}

/// The outlet over X: replies go out through an [`XPublisher`], so
/// `DRY_RUN` and the 280 cap hold here too. Its guard lets everything
/// through because the reply job has already run the content filter and the
/// house style on the text, as the legacy job did.
pub struct XOutlet {
    x: Arc<XApi>,
    publisher: XPublisher,
}

impl XOutlet {
    pub fn new(x: Arc<XApi>, dry_run: bool) -> Self {
        Self {
            publisher: XPublisher::new(Arc::clone(&x), Arc::new(NoopGuard), dry_run),
            x,
        }
    }
}

#[async_trait]
impl ReplyOutlet for XOutlet {
    async fn upload(&self, media: Media) -> anyhow::Result<String> {
        if self.publisher.is_dry_run() {
            anyhow::bail!("dry run: nothing is uploaded");
        }
        Ok(self.x.upload_media(&media).await?.media_id)
    }

    async fn reply(
        &self,
        text: &str,
        reply_to: &PostId,
        media_ids: Vec<String>,
    ) -> anyhow::Result<PostId> {
        let draft = Draft {
            reply_to: Some(reply_to.clone()),
            media_ids,
            ..Draft::new(text)
        };
        Ok(self.publisher.post(draft).await?.id)
    }

    async fn like(&self, post_id: &PostId) -> anyhow::Result<()> {
        Ok(self.publisher.like(post_id).await?)
    }
}

/// Records every call and answers `99` / `media-1`, or fails on request.
/// For tests and rehearsals.
#[derive(Default)]
pub struct RecordingOutlet {
    replies: Mutex<Vec<(String, String, Vec<String>)>>,
    uploads: Mutex<Vec<Media>>,
    likes: Mutex<Vec<String>>,
    failures: Mutex<Failures>,
}

#[derive(Default)]
struct Failures {
    reply: Option<String>,
    upload: Option<String>,
    like: Option<String>,
}

fn locked<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

impl RecordingOutlet {
    /// Every reply: text, the post replied to, the media ids.
    pub fn replies(&self) -> Vec<(String, String, Vec<String>)> {
        locked(&self.replies).clone()
    }

    pub fn uploads(&self) -> Vec<Media> {
        locked(&self.uploads).clone()
    }

    pub fn likes(&self) -> Vec<String> {
        locked(&self.likes).clone()
    }

    pub fn fail_replies(&self, message: &str) {
        locked(&self.failures).reply = Some(message.to_owned());
    }

    pub fn fail_uploads(&self, message: &str) {
        locked(&self.failures).upload = Some(message.to_owned());
    }

    pub fn fail_likes(&self, message: &str) {
        locked(&self.failures).like = Some(message.to_owned());
    }
}

#[async_trait]
impl ReplyOutlet for RecordingOutlet {
    async fn upload(&self, media: Media) -> anyhow::Result<String> {
        if let Some(message) = &locked(&self.failures).upload {
            anyhow::bail!("{message}");
        }
        locked(&self.uploads).push(media);
        Ok("media-1".into())
    }

    async fn reply(
        &self,
        text: &str,
        reply_to: &PostId,
        media_ids: Vec<String>,
    ) -> anyhow::Result<PostId> {
        if let Some(message) = &locked(&self.failures).reply {
            anyhow::bail!("{message}");
        }
        locked(&self.replies).push((text.to_owned(), reply_to.to_string(), media_ids));
        Ok(PostId::from("99"))
    }

    async fn like(&self, post_id: &PostId) -> anyhow::Result<()> {
        if let Some(message) = &locked(&self.failures).like {
            anyhow::bail!("{message}");
        }
        locked(&self.likes).push(post_id.to_string());
        Ok(())
    }
}
