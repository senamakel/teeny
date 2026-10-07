//! What an operator's decision on a hold actually does.
//!
//! Rejecting confirms the filter and closes the entry. Approving overrules
//! it, and an overrule that changes nothing is a note, not a decision, so
//! each kind of hold has an effect: a cut link puts its host on the live
//! allowlist, a held mention goes back to the queue, a withheld draft is
//! released as written. What cannot be acted on after the fact (a Telegram
//! message from an hour ago, a picture the account declined to draw) is
//! recorded as reviewed, and the effect says so.
//! A port of `legacy/src/safety/review.ts`.

use async_trait::async_trait;
use url::Url;

use teeny_store::StoreError;
use teeny_store::holds::{DecisionDetail, Hold, HoldDecision, HoldKind, HoldLog, HoldSurface};

/// A review that could not be carried out.
#[derive(Debug, thiserror::Error)]
pub enum ReviewError {
    #[error(transparent)]
    Log(#[from] StoreError),
    /// The approval's effect failed (a release that did not publish). The
    /// hold goes back to pending, the failure recorded, so it can be tried
    /// again.
    #[error("{0}")]
    Effect(String),
}

/// Puts an X mention back to be answered with the inbound check waived.
#[async_trait]
pub trait Requeue: Send + Sync {
    /// Whether the mention was still there to requeue.
    async fn requeue(&self, post_id: &str) -> Result<bool, ReviewError>;
}

/// Publishes a withheld draft as written.
#[async_trait]
pub trait Release: Send + Sync {
    /// A line on what happened, e.g. "published on X as 99".
    async fn release(&self, hold: &Hold) -> Result<String, ReviewError>;
}

/// What a review can reach.
pub struct ReviewDependencies<'a> {
    pub holds: &'a dyn HoldLog,
    pub requeue: Option<&'a dyn Requeue>,
    pub release: Option<&'a dyn Release>,
    /// Tells a gate in this process to re-read the approved links.
    pub refresh_links: Option<&'a (dyn Fn() + Sync)>,
}

/// A cut link as an allowlist entry: its host, with subdomains.
pub fn allowlist_entry_for(link: &str) -> Option<String> {
    let lower = link.get(..8).unwrap_or(link).to_ascii_lowercase();
    let absolute = if lower.starts_with("http://") || lower.starts_with("https://") {
        link.to_owned()
    } else {
        format!("https://{link}")
    };
    let host = Url::parse(&absolute).ok()?.host_str()?.to_lowercase();
    (!host.is_empty()).then_some(host)
}

/// Takes an operator's decision on a hold. `None` for an unknown id.
///
/// A decision is taken once: re-deciding would re-run the effect (a second
/// publish of a released draft, a second requeue), so the hold is claimed
/// first, atomically, and a hold that cannot be claimed (decided, or being
/// decided by a second click this instant) is answered as it stands.
///
/// A failed effect puts the claim back, so the hold can be tried again. Once
/// the effect has happened the hold is never pending again: a decision that
/// cannot be written is retried once with the failure in its note, and if
/// that fails too the hold stays `releasing`, visibly stuck rather than
/// offered for a second publish.
pub async fn review_hold(
    id: &str,
    decision: HoldDecision,
    deps: &ReviewDependencies<'_>,
    note: Option<&str>,
) -> Result<Option<Hold>, ReviewError> {
    let Some(hold) = deps.holds.claim(id).await? else {
        return Ok(deps.holds.get(id).await?);
    };
    let effect = match decision {
        HoldDecision::Rejected => "confirmed the filter".to_owned(),
        HoldDecision::Approved => match approve(&hold, deps).await {
            Ok(effect) => effect,
            Err(failure) => {
                if let Err(error) = deps.holds.unclaim(id, &failure.to_string()).await {
                    tracing::error!(hold_id = id, %error, "could not put a failed release back to pending");
                }
                return Err(failure);
            }
        },
    };
    let detail = DecisionDetail {
        note: note.map(str::to_owned),
        effect: Some(effect),
    };
    match deps.holds.decide(id, decision, &detail).await {
        Ok(decided) => Ok(decided),
        Err(first) => {
            tracing::warn!(hold_id = id, error = %first, "carried out a hold decision but could not record it; retrying");
            let late = format!("decision recorded late: {first}");
            let detail = DecisionDetail {
                note: Some(match note.filter(|note| !note.is_empty()) {
                    Some(note) => format!("{note} ({late})"),
                    None => late,
                }),
                ..detail
            };
            Ok(deps.holds.decide(id, decision, &detail).await?)
        }
    }
}

async fn approve(hold: &Hold, deps: &ReviewDependencies<'_>) -> Result<String, ReviewError> {
    match hold.kind {
        HoldKind::Link => {
            let mut hosts: Vec<String> = Vec::new();
            for cut in hold.links.iter().flatten() {
                if let Some(host) = allowlist_entry_for(&cut.link)
                    && !hosts.contains(&host)
                {
                    hosts.push(host);
                }
            }
            if hosts.is_empty() {
                return Ok("no usable host in the cut links; nothing allowlisted".into());
            }
            for host in &hosts {
                deps.holds.allow_link(host, &hold.id).await?;
            }
            if let Some(refresh) = deps.refresh_links {
                refresh();
            }
            Ok(format!("allowlisted {}", hosts.join(", ")))
        }
        HoldKind::Message => {
            let post_id = hold
                .context
                .post_id
                .as_deref()
                .filter(|_| hold.context.surface == HoldSurface::X);
            let Some(post_id) = post_id else {
                return Ok(
                    "recorded; a held chat message cannot be answered after the fact".into(),
                );
            };
            let Some(requeue) = deps.requeue else {
                return Ok("recorded; no queue to return it to".into());
            };
            Ok(if requeue.requeue(post_id).await? {
                "requeued to be answered".into()
            } else {
                "recorded; the mention is no longer in the queue".into()
            })
        }
        // A release that fails is an error, and the hold goes back to pending:
        // marking it approved with a failure note would be a decision that did
        // nothing, dressed as one that did.
        HoldKind::Draft => match deps.release {
            Some(release) => release.release(hold).await,
            None => Ok("recorded; nothing here can publish it".into()),
        },
        HoldKind::Prompt | HoldKind::Task => Ok("recorded".into()),
    }
}

#[cfg(test)]
#[path = "review_tests.rs"]
mod tests;
