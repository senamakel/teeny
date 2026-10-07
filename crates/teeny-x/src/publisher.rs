//! The one public way to publish. Every write goes guard, then the hard
//! 280-character check, then `DRY_RUN`, then X. The raw write calls on
//! [`XApi`] are crate-private, so nothing outside this crate can post around
//! the guard.
//!
//! teeny-safety and teeny-cadence implement [`PublishGuard`] (safety gate,
//! house style, cadence and budgets); [`NoopGuard`] is for tests.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use teeny_core::types::PostId;

use crate::article::{ArticleDraft, PublishedArticle};
use crate::client::XApi;
use crate::error::XError;
use crate::model::CreateOptions;

/// The hard cap, as X counts it for plain text.
pub const MAX_POST_LENGTH: usize = 280;

/// A post about to be published.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    pub text: String,
    pub reply_to: Option<PostId>,
    pub quote: Option<PostId>,
    pub media_ids: Vec<String>,
}

impl Draft {
    /// A standalone text post.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            reply_to: None,
            quote: None,
            media_ids: Vec::new(),
        }
    }

    /// Its length as the legacy service measured it: UTF-16 code units
    /// (JavaScript's `.length`), so an emoji counts two, as X counts it.
    pub fn length(&self) -> usize {
        self.text.encode_utf16().count()
    }
}

/// Why a guard would not let a draft out.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("refused: {reason}")]
pub struct Refusal {
    pub reason: String,
}

impl Refusal {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

/// Checks a draft before it is published; may return it rewritten.
#[async_trait]
pub trait PublishGuard: Send + Sync {
    async fn check(&self, draft: &Draft) -> Result<Draft, Refusal>;
}

/// Lets everything through unchanged. For tests.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopGuard;

#[async_trait]
impl PublishGuard for NoopGuard {
    async fn check(&self, draft: &Draft) -> Result<Draft, Refusal> {
        Ok(draft.clone())
    }
}

/// Why a post was not published.
#[derive(Debug, thiserror::Error)]
pub enum PublishError {
    #[error(transparent)]
    Refused(#[from] Refusal),
    #[error("the post is {length} characters; X allows {MAX_POST_LENGTH}")]
    TooLong { length: usize },
    #[error(transparent)]
    X(#[from] XError),
}

/// A post that went out, or would have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Published {
    /// X's id, or a `dry-run-` placeholder.
    pub id: PostId,
    pub dry_run: bool,
    /// The text as it went out (or would have): the guard's rewrite.
    pub text: String,
}

/// Publishes as the account, through a guard, honouring `DRY_RUN`.
pub struct XPublisher {
    x: Arc<XApi>,
    guard: Arc<dyn PublishGuard>,
    dry_run: bool,
}

fn dry_run_id() -> PostId {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0);
    PostId::from(format!("dry-run-{millis}"))
}

impl XPublisher {
    /// `dry_run` is `Config::dry_run`: on, nothing reaches X.
    pub fn new(x: Arc<XApi>, guard: Arc<dyn PublishGuard>, dry_run: bool) -> Self {
        Self { x, guard, dry_run }
    }

    pub fn is_dry_run(&self) -> bool {
        self.dry_run
    }

    /// Guard, length check, `DRY_RUN`, post.
    pub async fn post(&self, draft: Draft) -> Result<Published, PublishError> {
        self.post_with_media(draft, || async { Ok(Vec::new()) })
            .await
    }

    /// [`post`](Self::post), with the attachments made by `attach` only once
    /// the guard has passed the draft and only when the post is real: a
    /// refused or rehearsed post uploads nothing. The ids `attach` returns
    /// are added to the draft's own.
    pub async fn post_with_media<F, Fut>(
        &self,
        draft: Draft,
        attach: F,
    ) -> Result<Published, PublishError>
    where
        F: FnOnce() -> Fut + Send,
        Fut: Future<Output = Result<Vec<String>, PublishError>> + Send,
    {
        let draft = self.guard.check(&draft).await?;
        let length = draft.length();
        if length > MAX_POST_LENGTH {
            return Err(PublishError::TooLong { length });
        }
        if self.dry_run {
            tracing::info!(text = draft.text, reply_to = ?draft.reply_to, quote = ?draft.quote, "dry run: would have posted on X");
            return Ok(Published {
                id: dry_run_id(),
                dry_run: true,
                text: draft.text,
            });
        }
        let mut media_ids = draft.media_ids;
        media_ids.extend(attach().await?);
        let options = CreateOptions {
            reply_to: draft.reply_to,
            quote: draft.quote,
            media_ids,
        };
        let created = self.x.create_post(&draft.text, &options).await?;
        Ok(Published {
            id: created.id,
            dry_run: false,
            text: draft.text,
        })
    }

    /// Deletes one of the account's posts. `false` in a dry run, or when X
    /// did not confirm it.
    pub async fn unsay(&self, post_id: &PostId) -> Result<bool, XError> {
        if self.dry_run {
            tracing::info!(%post_id, "dry run: would have deleted the post");
            return Ok(false);
        }
        self.x.delete_post(post_id).await
    }

    /// Likes a post as the account; a no-op in a dry run.
    pub async fn like(&self, post_id: &PostId) -> Result<(), XError> {
        if self.dry_run {
            tracing::info!(%post_id, "dry run: would have liked the post");
            return Ok(());
        }
        self.x.like_post(post_id).await
    }

    /// Drafts and publishes an X Article. In a dry run nothing reaches X and
    /// both ids are placeholders. The article's own safety review happens in
    /// its pipeline; it is not a 280-character draft.
    pub async fn publish_article(&self, draft: &ArticleDraft) -> Result<PublishedArticle, XError> {
        if self.dry_run {
            tracing::info!(
                title = draft.title,
                "dry run: would have published the article on X"
            );
            let id = dry_run_id();
            return Ok(PublishedArticle {
                article_id: id.to_string(),
                post_id: id,
            });
        }
        self.x.publish_article(draft).await
    }
}

#[cfg(test)]
#[path = "publisher_tests.rs"]
mod tests;
