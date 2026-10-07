//! The account's own post, as every daily pipeline publishes it: through
//! [`Cadence::publish_own`] (claimed before, recorded after) and an
//! [`XPublisher`] (the guard, the 280 cap, `DRY_RUN`). A dry run never
//! reaches X and never enters the post ledger, so a rehearsal cannot hold
//! the next real post back for the gap.
//!
//! [`OwnPoster`] is the seam the pipelines post through; [`XOwnPoster`] is
//! the real one.

use std::sync::Arc;

use async_trait::async_trait;
use teeny_agent::deps::Deps;
use teeny_agent::tools::post::PostingGuard;
use teeny_cadence::cadence::{Cadence, OwnPostId, PublishError as PacedError};
use teeny_core::types::PostId;
use teeny_media::fit::fit_image_for_x;
pub use teeny_media::types::MimeType;
use teeny_store::holds::HoldSurface;
use teeny_x::model::{Media, MediaType};
use teeny_x::{Draft, PublishError, Published, XApi, XPublisher};

/// A picture or a clip to attach.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    pub data: Vec<u8>,
    pub mime_type: MimeType,
}

/// One own post: never a reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnPost {
    /// The cadence's name for it (`morning`, `moon`, `quote`...).
    pub kind: String,
    pub text: String,
    pub media: Option<Attachment>,
    /// The post this one quotes.
    pub quote: Option<PostId>,
}

/// What went out, or would have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Posted {
    /// X's id, or a `dry-run-` placeholder.
    pub id: String,
    /// The text as the guard let it out.
    pub text: String,
    pub dry_run: bool,
}

/// Why nothing went out.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PostFailure {
    /// The cadence said not now.
    #[error("{0}")]
    Held(String),
    /// The guard refused the draft.
    #[error("{0}")]
    Refused(String),
    /// X, or the ledger, failed.
    #[error("{0}")]
    Failed(String),
}

/// Publishes an own post.
#[async_trait]
pub trait OwnPoster: Send + Sync {
    async fn post(&self, post: OwnPost) -> Result<Posted, PostFailure>;
}

/// The real poster: the cadence in front of the publisher.
pub struct XOwnPoster {
    x: Arc<XApi>,
    publisher: Arc<XPublisher>,
    cadence: Arc<Cadence>,
}

impl XOwnPoster {
    pub fn new(x: Arc<XApi>, publisher: Arc<XPublisher>, cadence: Arc<Cadence>) -> Self {
        Self {
            x,
            publisher,
            cadence,
        }
    }

    /// From the clients: the content filter and house style as the guard
    /// (holds filed under `surface` and `tool`), and `DRY_RUN`.
    pub fn from_deps(deps: &Deps, surface: HoldSurface, tool: &'static str) -> Self {
        let guard = PostingGuard::new(
            Arc::clone(&deps.safety),
            surface,
            tool,
            deps.settings.max_emoji,
        );
        Self::new(
            Arc::clone(&deps.x),
            Arc::new(XPublisher::new(
                Arc::clone(&deps.x),
                Arc::new(guard),
                deps.settings.dry_run,
            )),
            Arc::clone(&deps.cadence),
        )
    }
}

/// A published post, as the ledger names it.
struct Out(Published);

impl OwnPostId for Out {
    fn own_post_id(&self) -> &str {
        self.0.id.as_str()
    }
}

fn x_media_type(mime: MimeType) -> MediaType {
    match mime {
        MimeType::Png => MediaType::Png,
        MimeType::Jpeg => MediaType::Jpeg,
        MimeType::Webp => MediaType::Webp,
        MimeType::Gif => MediaType::Gif,
        MimeType::Mp4 => MediaType::Mp4,
    }
}

fn refused(error: PublishError) -> PostFailure {
    match error {
        PublishError::Refused(refusal) => PostFailure::Refused(refusal.reason),
        other => PostFailure::Failed(other.to_string()),
    }
}

fn posted(published: Published) -> Posted {
    Posted {
        id: published.id.to_string(),
        text: published.text,
        dry_run: published.dry_run,
    }
}

#[async_trait]
impl OwnPoster for XOwnPoster {
    async fn post(&self, post: OwnPost) -> Result<Posted, PostFailure> {
        let draft = Draft {
            quote: post.quote.clone(),
            ..Draft::new(&post.text)
        };
        if self.publisher.is_dry_run() {
            let published = self
                .publisher
                .post(draft)
                .await
                .map_err(refused)?;
            return Ok(posted(published));
        }
        let x = Arc::clone(&self.x);
        let attach = move || async move {
            let Some(media) = post.media else {
                return Ok(Vec::new());
            };
            // A big still is re-encoded under X's limit; a clip goes as it is.
            let (data, mime) = match media.mime_type {
                MimeType::Mp4 | MimeType::Gif => (media.data, media.mime_type),
                still => fit_image_for_x(media.data, still).map_err(|error| {
                    PublishError::Refused(teeny_x::Refusal::new(format!(
                        "the picture could not be fitted for X ({error})"
                    )))
                })?,
            };
            let uploaded = x
                .upload_media(&Media {
                    data,
                    mime: x_media_type(mime),
                })
                .await?;
            Ok(vec![uploaded.media_id])
        };
        let publisher = Arc::clone(&self.publisher);
        let out = self
            .cadence
            .publish_own(&post.kind, Some(&post.text), || async move {
                publisher.post_with_media(draft, attach).await.map(Out)
            })
            .await
            .map_err(|error| match error {
                PacedError::Held(held) => PostFailure::Held(held.to_string()),
                PacedError::Ledger(error) => PostFailure::Failed(format!("cadence: {error}")),
                PacedError::Publish(error) => refused(error),
            })?;
        Ok(posted(out.0))
    }
}

#[cfg(test)]
#[path = "post_tests.rs"]
mod tests;
