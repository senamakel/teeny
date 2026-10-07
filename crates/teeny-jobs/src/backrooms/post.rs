//! The one way the long-form side publishes a post: the job's last step,
//! through an [`XPublisher`] (its guard, the 280 cap, `DRY_RUN`) inside the
//! cadence ([`Cadence::publish_own`](teeny_cadence::cadence::Cadence::publish_own)),
//! with any attachment uploaded only once the post is real.
//!
//! The pipelines run the full outbound gate themselves before they get
//! here, because a hold is an outcome they journal with its category; the
//! publisher's guard re-reads the final text against the lexicon, cheaply,
//! so nothing that slipped between the two leaves.

use std::sync::Arc;

use async_trait::async_trait;
use teeny_agent::deps::Deps;
use teeny_cadence::cadence::{OwnPostId, PublishError as CadenceError};
use teeny_safety::gate::SafetyGate;
use teeny_safety::screen::Direction;
use teeny_x::model::Media;
use teeny_x::{Draft, PublishError, PublishGuard, Published, Refusal, XPublisher};

/// Refuses a draft the lexicon holds under `direction`.
pub struct ScreenGuard {
    pub safety: Arc<SafetyGate>,
    pub direction: Direction,
}

#[async_trait]
impl PublishGuard for ScreenGuard {
    async fn check(&self, draft: &Draft) -> Result<Draft, Refusal> {
        if self.safety.screen(&draft.text, self.direction) {
            Ok(draft.clone())
        } else {
            Err(Refusal::new("the content filter holds this post"))
        }
    }
}

struct Out(Published);

impl OwnPostId for Out {
    fn own_post_id(&self) -> &str {
        self.0.id.as_str()
    }
}

/// Publishes `text` (with `media` attached) as an own post of `kind`. In a
/// dry run nothing reaches X, nothing is uploaded and the post ledger is
/// left alone; the id is a `dry-run-` placeholder.
pub async fn publish_own(
    deps: &Deps,
    kind: &str,
    direction: Direction,
    text: &str,
    media: Option<Media>,
) -> anyhow::Result<Published> {
    let publisher = XPublisher::new(
        Arc::clone(&deps.x),
        Arc::new(ScreenGuard {
            safety: Arc::clone(&deps.safety),
            direction,
        }),
        deps.settings.dry_run,
    );
    let x = Arc::clone(&deps.x);
    let post = async || {
        publisher
            .post_with_media(Draft::new(text), || async move {
                match media {
                    Some(media) => Ok(vec![x.upload_media(&media).await?.media_id]),
                    None => Ok(Vec::new()),
                }
            })
            .await
    };
    if publisher.is_dry_run() {
        return Ok(post().await?);
    }
    match deps
        .cadence
        .publish_own(kind, Some(text), async || post().await.map(Out))
        .await
    {
        Ok(Out(published)) => Ok(published),
        Err(CadenceError::Held(held)) => Err(anyhow::anyhow!(held.to_string())),
        Err(CadenceError::Ledger(error)) => Err(anyhow::anyhow!("cadence: {error}")),
        Err(CadenceError::Publish(error)) => Err(publish_error(error)),
    }
}

fn publish_error(error: PublishError) -> anyhow::Error {
    anyhow::anyhow!(error.to_string())
}

#[cfg(test)]
#[path = "post_tests.rs"]
mod tests;
