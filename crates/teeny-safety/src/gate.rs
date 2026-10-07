//! The gate every message passes through twice: once on the way in, once on
//! the way out.
//!
//! Inbound, a prohibited message is not answered: not declined, not
//! deflected, not processed. The drafter never sees it, nothing about it is
//! remembered, and the journal records that something was held and why,
//! without the text.
//!
//! Outbound, the account's own draft is held to a stricter reading
//! (profanity counts, and every link is checked against the allowlist), and
//! the caller gets back either clean text to publish or a reason not to. A
//! clean question can still get a dirty answer, from a model with a bad day
//! or a web page with a plan.
//!
//! Checks run cheapest first and each is sufficient to stop the turn: links
//! are cut, the lexicon is read, and only then is the judge asked.
//! A port of `legacy/src/safety/gate.ts`, plus [`SafetyGate::check_outbound`],
//! the seam the X Publisher guard calls (the legacy `x_post` tool's checks).

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::holds::{HeldBy, HoldContext, HoldInput, HoldKind, HoldLog, HoldSurface};
use crate::house_style::{HouseStyleOptions, MAX_POST_LENGTH, apply_house_style, count_emoji, post_length};
use crate::judge::{Judge, Judgement};
use crate::links::{AllowlistEntry, DEFAULT_LINK_ALLOWLIST, filter_links, parse_allowlist};
use crate::screen::{Direction, screen_text};

/// Why a text was held.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Held {
    /// A lexicon category, the judge's category, `links`, or `unknown`.
    pub category: String,
    /// For the log and the journal. Never contains the text.
    pub reason: String,
    pub by: HeldBy,
}

/// The gate's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SafetyDecision {
    Clean,
    Held(Held),
}

impl SafetyDecision {
    pub fn is_clean(&self) -> bool {
        matches!(self, Self::Clean)
    }

    fn held(category: impl Into<String>, reason: impl Into<String>, by: HeldBy) -> Self {
        Self::Held(Held { category: category.into(), reason: reason.into(), by })
    }
}

/// An outbound check: the decision, and the text with disallowed links cut.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundResult {
    pub decision: SafetyDecision,
    /// What to publish. Only meaningful when the decision is clean.
    pub text: String,
    /// What was changed, for logging. Empty when the draft was already clean.
    pub fixes: Vec<String>,
}

/// How a gate is built.
#[derive(Clone)]
pub struct SafetyGateOptions {
    /// Absent, only the lexicon and the allowlist run. Present, asked last.
    pub judge: Option<Arc<dyn Judge>>,
    /// Added to the built-in allowlist (`LINK_ALLOWLIST`), `host[/path]`.
    pub extra_allowed_links: Vec<String>,
    /// Where holds are written, and operator-approved links read from.
    /// Absent, holds are logged and forgotten.
    pub holds: Option<Arc<dyn HoldLog>>,
    /// How long approved links are cached before the log is asked again.
    pub approved_links_ttl: Duration,
}

impl Default for SafetyGateOptions {
    fn default() -> Self {
        Self {
            judge: None,
            extra_allowed_links: Vec::new(),
            holds: None,
            approved_links_ttl: Duration::from_secs(60),
        }
    }
}

#[derive(Default)]
struct Approved {
    entries: Vec<AllowlistEntry>,
    /// When the log was last asked; `None` asks on the next check.
    at: Option<Instant>,
}

/// A draft the account wants to publish through a tool, with the model still
/// in the loop to rewrite it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundDraft {
    pub text: String,
    /// Where it is going, for the hold if there is one.
    pub context: HoldContext,
    /// `MAX_EMOJI_PER_POST`.
    pub max_emoji: usize,
    /// The handle being replied to, whose `@` X adds by itself.
    pub replying_to: Option<String>,
}

impl OutboundDraft {
    /// A draft under the default emoji cap of one, replying to nobody.
    pub fn new(text: impl Into<String>, context: HoldContext) -> Self {
        Self { text: text.into(), context, max_emoji: 1, replying_to: None }
    }
}

/// What may happen to an outbound draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Publish this text: the house style applied and the gate passed.
    Allow(String),
    /// The gate held it, and recorded it for review. Do not rephrase it.
    Hold(String),
    /// It cannot go out as written (too long, too many emoji, a link off the
    /// allowlist); the model may rewrite it and try again.
    Refuse(String),
}

/// The content filter.
pub struct SafetyGate {
    judge: Option<Arc<dyn Judge>>,
    configured: Vec<AllowlistEntry>,
    holds: Option<Arc<dyn HoldLog>>,
    ttl: Duration,
    approved: Mutex<Approved>,
}

impl SafetyGate {
    pub fn new(options: SafetyGateOptions) -> Self {
        let configured = parse_allowlist(
            DEFAULT_LINK_ALLOWLIST.iter().copied().chain(options.extra_allowed_links.iter().map(String::as_str)),
        );
        Self {
            judge: options.judge,
            configured,
            holds: options.holds,
            ttl: options.approved_links_ttl,
            approved: Mutex::new(Approved::default()),
        }
    }

    /// The built-in list plus whatever an operator has approved since, read
    /// on a short cache so an approval taken in the API process reaches the
    /// reply process without a restart. A log that cannot be read leaves the
    /// approved set as it was, never empties it.
    async fn allowlist(&self) -> Vec<AllowlistEntry> {
        if let Some(holds) = &self.holds {
            let stale = self.approved.lock().expect("not poisoned").at.is_none_or(|at| at.elapsed() > self.ttl);
            if stale {
                let read = holds.allowed_links().await;
                let mut approved = self.approved.lock().expect("not poisoned");
                match read {
                    Ok(entries) => approved.entries = parse_allowlist(entries),
                    Err(error) => {
                        tracing::warn!(%error, "safety: could not read approved links; keeping the last set");
                    }
                }
                approved.at = Some(Instant::now());
            }
        }
        let approved = self.approved.lock().expect("not poisoned");
        self.configured.iter().chain(approved.entries.iter()).cloned().collect()
    }

    /// Forgets the cached approvals, so the next check reads the log.
    pub fn refresh_approved_links(&self) {
        self.approved.lock().expect("not poisoned").at = None;
    }

    /// Writes a hold for review. Never fails: the decision is already made.
    async fn hold(&self, entry: HoldInput) {
        let Some(holds) = &self.holds else { return };
        if let Err(error) = holds.record(entry).await {
            tracing::warn!(%error, "safety: could not record a hold");
        }
    }

    /// The lexicon alone, synchronous, for scrubbing context lines cheaply.
    pub fn screen(&self, text: &str, direction: Direction) -> bool {
        screen_text(text, direction).is_none()
    }

    /// Whether a message from somebody else is something the account
    /// processes at all. With a context, a hold is recorded for review.
    pub async fn inbound(&self, text: &str, context: Option<&HoldContext>) -> SafetyDecision {
        let decision = match screen_text(text, Direction::Inbound) {
            Some(found) => {
                tracing::info!(category = %found.category, term = found.term, "safety: inbound held by the screen");
                SafetyDecision::held(found.category.as_str(), format!("contains \"{}\"", found.term), HeldBy::Screen)
            }
            None => self.ask(text, Direction::Inbound).await,
        };
        if let (SafetyDecision::Held(held), Some(context)) = (&decision, context) {
            let kind = context.kind.unwrap_or(if context.surface == HoldSurface::Task {
                HoldKind::Task
            } else {
                HoldKind::Message
            });
            self.hold(HoldInput {
                direction: Direction::Inbound,
                kind,
                context: context.clone(),
                category: held.category.clone(),
                reason: held.reason.clone(),
                by: held.by,
                text: text.to_owned(),
                links: None,
            })
            .await;
        }
        decision
    }

    /// The account's own words, checked before they leave. The text handed
    /// back has its links filtered; the caller publishes that. A cut link is
    /// a hold of its own even when the rest goes out, because it is the
    /// decision a reviewer most often wants to overturn.
    pub async fn outbound(&self, text: &str, context: Option<&HoldContext>) -> OutboundResult {
        let mut fixes = Vec::new();
        let links = filter_links(text, &self.allowlist().await);
        let cut = !links.removed.is_empty();
        if cut {
            fixes.push(format!("cut {} link(s) not on the allowlist", links.removed.len()));
            tracing::warn!(removed = ?links.removed, "safety: cut links from an outbound draft");
        }

        // The serial's posts and the articles written from its lore are read
        // under the narrative profile; every other draft under the strict one.
        let direction = match context.map(|c| c.surface) {
            Some(HoldSurface::Narrative) => Direction::Narrative,
            Some(HoldSurface::Article) => Direction::Article,
            _ => Direction::Outbound,
        };
        let decision = self.outbound_decision(&links.text, cut, direction).await;
        if let Some(context) = context {
            let removed = cut.then(|| links.removed.clone());
            match &decision {
                SafetyDecision::Held(held) => {
                    self.hold(HoldInput {
                        direction: Direction::Outbound,
                        kind: context.kind.unwrap_or(HoldKind::Draft),
                        context: context.clone(),
                        category: held.category.clone(),
                        reason: held.reason.clone(),
                        by: held.by,
                        text: text.to_owned(),
                        links: removed,
                    })
                    .await;
                }
                SafetyDecision::Clean if cut => {
                    let reason = links.removed.iter().map(|r| r.reason.as_str()).collect::<Vec<_>>().join("; ");
                    self.hold(HoldInput {
                        direction: Direction::Outbound,
                        kind: HoldKind::Link,
                        context: context.clone(),
                        category: "links".into(),
                        reason,
                        by: HeldBy::Links,
                        text: text.to_owned(),
                        links: removed,
                    })
                    .await;
                }
                SafetyDecision::Clean => {}
            }
        }
        OutboundResult { decision, text: links.text, fixes }
    }

    async fn outbound_decision(&self, cleaned: &str, links_cut: bool, direction: Direction) -> SafetyDecision {
        if let Some(found) = screen_text(cleaned, direction) {
            tracing::warn!(category = %found.category, term = found.term, "safety: outbound held by the screen");
            return SafetyDecision::held(found.category.as_str(), format!("contains \"{}\"", found.term), HeldBy::Screen);
        }
        // Nothing left to say once the links are gone is not a message to send.
        if links_cut && !cleaned.chars().any(|c| c.is_ascii_alphanumeric()) {
            return SafetyDecision::held("links", "the draft was only a disallowed link", HeldBy::Links);
        }
        self.ask(cleaned, direction).await
    }

    async fn ask(&self, text: &str, direction: Direction) -> SafetyDecision {
        let Some(judge) = &self.judge else { return SafetyDecision::Clean };
        if text.trim().is_empty() {
            return SafetyDecision::Clean;
        }
        let verdict = judge.judge(text, direction).await;
        if verdict.verdict == Judgement::Clean {
            return SafetyDecision::Clean;
        }
        let category = verdict.category.unwrap_or_else(|| "unknown".into());
        if direction == Direction::Inbound {
            tracing::info!(direction = direction.as_str(), %category, reason = %verdict.reason, "safety: held by the judge");
        } else {
            tracing::warn!(direction = direction.as_str(), %category, reason = %verdict.reason, "safety: held by the judge");
        }
        let reason = if verdict.reason.is_empty() { "the judge held it".to_owned() } else { verdict.reason };
        SafetyDecision::held(category, reason, HeldBy::Judge)
    }

    /// The checks a posting tool runs before anything goes out, in the legacy
    /// `x_post` order: the hard limits are refused so the model learns from
    /// them, the rest of the house style is applied silently, and the result
    /// goes through [`outbound`](Self::outbound) with the draft's context.
    pub async fn check_outbound(&self, draft: &OutboundDraft) -> Verdict {
        let length = post_length(&draft.text);
        if length > MAX_POST_LENGTH {
            return Verdict::Refuse(format!("the text is {length} characters, over the {MAX_POST_LENGTH} limit."));
        }
        let emoji = count_emoji(&draft.text);
        if emoji > draft.max_emoji {
            return Verdict::Refuse(format!(
                "the text has {emoji} emoji and this account allows at most {} per post. Rewrite it and call the tool again.",
                draft.max_emoji
            ));
        }
        let styled = apply_house_style(
            &draft.text,
            &HouseStyleOptions {
                max_emoji: draft.max_emoji,
                max_length: MAX_POST_LENGTH,
                replying_to: draft.replying_to.clone(),
            },
        );
        let result = self.outbound(&styled.text, Some(&draft.context)).await;
        if let SafetyDecision::Held(held) = result.decision {
            return Verdict::Hold(format!(
                "this account does not publish that ({}). Do not rephrase it; say something else or say nothing.",
                held.category
            ));
        }
        if !result.fixes.is_empty() {
            return Verdict::Refuse(
                "the text links to a site that is not on the allowlist. This account only links to its own sites, \
                 code and community. Drop the link and call the tool again."
                    .into(),
            );
        }
        Verdict::Allow(result.text)
    }
}

#[cfg(test)]
#[path = "gate_tests.rs"]
mod tests;
