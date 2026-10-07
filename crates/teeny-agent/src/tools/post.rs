//! `x_post`, and the guard every agent-made post goes through.
//!
//! Publishing is host code, not prompt: the tool hands its draft to an
//! [`XPublisher`] whose [`PostingGuard`] runs, in order, the content filter
//! and house style ([`SafetyGate::check_outbound`]), then the cadence for an
//! own post or the reply budget for a reply ([`Pacing`]); the publisher then
//! applies the 280 cap and `DRY_RUN`, and only a real post uploads anything.
//! Every "no" comes back as a tool error in words the model can act on:
//! rewrite, drop the link, or stop. A port of the `x_post` tool in
//! `legacy/src/agent/tools/x.ts`.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;
use teeny_cadence::budget::{Mention, ReplyBudget};
use teeny_cadence::cadence::{Cadence, CadenceHeld, Claim};
use teeny_core::types::PostId;
use teeny_media::giphy::GifSource;
use teeny_media::store::{MediaStore, StoredMedia, is_media_id};
use teeny_media::types::{MediaKind, MimeType};
use teeny_safety::gate::{OutboundDraft, SafetyGate, Verdict};
use teeny_store::holds::{HoldContext, HoldKind, HoldSurface};
use teeny_x::cache::XReads;
use teeny_x::model::{Media, MediaType};
use teeny_x::{Draft, PublishError, PublishGuard, Published, Refusal, XApi, XPublisher};

use super::{Access, HostTool, Tool, ToolResult, answer, failed, invalid, is_x_id, parse};

/// Checks a draft before it is published: the content filter and house
/// style, then the pacing. Built per tool, so a hold says which surface and
/// which tool it came from.
pub struct PostingGuard {
    safety: Arc<SafetyGate>,
    surface: HoldSurface,
    tool: &'static str,
    max_emoji: usize,
    pacing: Option<Arc<Pacing>>,
}

impl PostingGuard {
    /// `max_emoji` is `MAX_EMOJI_PER_POST`.
    pub fn new(
        safety: Arc<SafetyGate>,
        surface: HoldSurface,
        tool: &'static str,
        max_emoji: usize,
    ) -> Self {
        Self {
            safety,
            surface,
            tool,
            max_emoji,
            pacing: None,
        }
    }

    /// The cadence and reply budget, after the filter.
    pub fn with_pacing(mut self, pacing: Arc<Pacing>) -> Self {
        self.pacing = Some(pacing);
        self
    }
}

#[async_trait]
impl PublishGuard for PostingGuard {
    async fn check(&self, draft: &Draft) -> Result<Draft, Refusal> {
        let context = HoldContext {
            kind: Some(HoldKind::Draft),
            post_id: draft.reply_to.as_ref().map(ToString::to_string),
            tool: Some(self.tool.to_owned()),
            ..HoldContext::new(self.surface)
        };
        let outbound = OutboundDraft {
            max_emoji: self.max_emoji,
            ..OutboundDraft::new(&draft.text, context)
        };
        let text = match self.safety.check_outbound(&outbound).await {
            Verdict::Allow(text) => text,
            Verdict::Hold(reason) | Verdict::Refuse(reason) => return Err(Refusal::new(reason)),
        };
        if let Some(pacing) = &self.pacing
            && let Some(reason) = pacing.hold(draft).await
        {
            return Err(Refusal::new(reason));
        }
        Ok(Draft {
            text,
            ..draft.clone()
        })
    }
}

/// Whether a reply may go out now: the reply budget's say.
#[async_trait]
pub trait ReplyAllowance: Send + Sync {
    /// Why a reply to `reply_to` may not go out, or `None` when it may.
    async fn refusal(&self, reply_to: &PostId) -> Option<String>;
}

/// The reply budget, asked about the post being replied to.
pub struct BudgetAllowance {
    reads: Arc<dyn XReads>,
    budget: Arc<ReplyBudget>,
    x: Arc<XApi>,
}

impl BudgetAllowance {
    /// `x` names the account, so a reply under its own post is told apart.
    pub fn new(reads: Arc<dyn XReads>, budget: Arc<ReplyBudget>, x: Arc<XApi>) -> Self {
        Self { reads, budget, x }
    }
}

#[async_trait]
impl ReplyAllowance for BudgetAllowance {
    async fn refusal(&self, reply_to: &PostId) -> Option<String> {
        // A post X cannot show is X's to refuse; the budget has nothing to count.
        let post = match self.reads.get_post(reply_to).await {
            Ok(Some(post)) => post,
            Ok(None) => return None,
            Err(error) => {
                return Some(format!(
                    "the post being replied to could not be read ({error}), so the reply budget could not be checked. Nothing was published."
                ));
            }
        };
        let account = match self.x.user_id().await {
            Ok(account) => account,
            Err(error) => {
                return Some(format!(
                    "the account could not be identified ({error}), so the reply budget could not be checked. Nothing was published."
                ));
            }
        };
        let mention = Mention {
            text: post.text,
            conversation_id: post.conversation_id.map(|id| id.to_string()),
            in_reply_to_user_id: post.in_reply_to_user_id.map(|id| id.to_string()),
        };
        match self.budget.assess(&mention, account.as_str()).await {
            Ok(assessment) if assessment.allowed => None,
            Ok(assessment) => Some(format!(
                "the {} reply budget is used up ({} of {}). Nothing was published; do not reply again until it has room.",
                format!("{:?}", assessment.lane).to_lowercase(),
                assessment.used,
                assessment
                    .cap
                    .map_or_else(|| "unlimited".to_owned(), |cap| cap.to_string()),
            )),
            Err(error) => Some(format!(
                "the reply budget could not be read ({error}). Nothing was published."
            )),
        }
    }
}

/// The cadence for the account's own posts, and the reply budget for its
/// replies. Replies are conversations, not slots, so the gap never holds one.
pub struct Pacing {
    cadence: Arc<Cadence>,
    kind: String,
    replies: Option<Arc<dyn ReplyAllowance>>,
}

impl Pacing {
    /// Own posts are recorded in the post ledger as `kind`.
    pub fn new(cadence: Arc<Cadence>, kind: &str) -> Self {
        Self {
            cadence,
            kind: kind.to_owned(),
            replies: None,
        }
    }

    /// The reply budget.
    pub fn with_replies(mut self, replies: Arc<dyn ReplyAllowance>) -> Self {
        self.replies = Some(replies);
        self
    }

    /// Why `draft` may not go out now, or `None`.
    pub async fn hold(&self, draft: &Draft) -> Option<String> {
        match &draft.reply_to {
            Some(reply_to) => match &self.replies {
                Some(replies) => replies.refusal(reply_to).await,
                None => None,
            },
            None => match self.cadence.claim(&self.kind).await {
                Ok(Claim::Granted) => None,
                Ok(claim) => Some(format!(
                    "{}. Nothing was published; do not post again before then.",
                    CadenceHeld(claim)
                )),
                Err(error) => Some(format!(
                    "the post ledger could not be read ({error}), so nothing was published."
                )),
            },
        }
    }

    /// Records an own post that went out, so the next one keeps the gap. A
    /// ledger that cannot be written is logged: the post is already out.
    pub async fn record(&self, published: &Published, reply_to: Option<&PostId>) {
        if published.dry_run || reply_to.is_some() {
            return;
        }
        if let Err(error) = self
            .cadence
            .record(published.id.as_str(), &self.kind, Some(&published.text))
            .await
        {
            tracing::warn!(%error, post_id = %published.id, "x_post: published but could not record the post");
        }
    }
}

/// Everything `x_post` needs.
pub struct XPostTool {
    /// Built with a [`PostingGuard`] for the `x_post` tool.
    pub publisher: Arc<XPublisher>,
    /// For uploads; posting itself only goes through the publisher.
    pub x: Arc<XApi>,
    /// Where generate_image / generate_video leave their files.
    pub media: Option<Arc<MediaStore>>,
    /// The account's GIF channel. Without it, `gifId` is refused.
    pub gifs: Option<Arc<dyn GifSource>>,
    pub pacing: Option<Arc<Pacing>>,
    /// `MAX_EMOJI_PER_POST`, named in the description.
    pub max_emoji: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PostArgs {
    text: String,
    reply_to_post_id: Option<String>,
    quote_post_id: Option<String>,
    gif_id: Option<String>,
    media_id: Option<String>,
}

/// What a dry run would have posted, in the legacy JSON order.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WouldHave<'a> {
    text: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    reply_to_post_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    quote_post_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gif_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    media_id: Option<&'a str>,
}

/// The X upload type of a stored file.
pub(crate) fn x_media_type(mime: MimeType) -> MediaType {
    match mime {
        MimeType::Png => MediaType::Png,
        MimeType::Jpeg => MediaType::Jpeg,
        MimeType::Webp => MediaType::Webp,
        MimeType::Gif => MediaType::Gif,
        MimeType::Mp4 => MediaType::Mp4,
    }
}

/// Uploads one attachment, for the publisher's attach step.
pub(crate) async fn upload(
    x: &XApi,
    data: Vec<u8>,
    mime: MediaType,
) -> Result<Vec<String>, PublishError> {
    let uploaded = x.upload_media(&Media { data, mime }).await?;
    Ok(vec![uploaded.media_id])
}

/// A publisher's "no", as the tool answers it.
pub(crate) fn refusal_of(error: PublishError) -> ToolResult {
    match error {
        PublishError::Refused(refusal) => answer(format!("Refused: {}", refusal.reason)),
        PublishError::TooLong { length } => answer(format!(
            "Refused: the text is {length} characters, over the {} limit.",
            teeny_x::MAX_POST_LENGTH
        )),
        PublishError::X(error) => failed(error),
    }
}

fn check_args(args: &PostArgs) -> Result<(), ToolResult> {
    if args.text.is_empty() {
        return Err(invalid("text must not be empty"));
    }
    for (field, id) in [
        ("replyToPostId", &args.reply_to_post_id),
        ("quotePostId", &args.quote_post_id),
    ] {
        if id.as_deref().is_some_and(|id| !is_x_id(id)) {
            return Err(invalid(format!("{field} must be a numeric X id")));
        }
    }
    if args
        .gif_id
        .as_deref()
        .is_some_and(|id| id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric()))
    {
        return Err(invalid("gifId must be a GIF id returned by find_gif"));
    }
    if args.media_id.as_deref().is_some_and(|id| !is_media_id(id)) {
        return Err(invalid(
            "mediaId must be a mediaId returned by generate_image or generate_video",
        ));
    }
    Ok(())
}

/// `x_post`: publish, reply or quote, with a GIF or a generated file.
pub fn x_post(tool: XPostTool) -> Arc<dyn Tool> {
    let description = format!(
        "Publish a post on X. Omit replyToPostId for a standalone post, set it to reply to someone, set \
         quotePostId to quote a post. Only reply to posts that mention the bot or quote its posts. Post once per \
         task unless explicitly asked for more. At most {} emoji per post.",
        tool.max_emoji
    );
    let tool = Arc::new(tool);
    HostTool::build(
        "x_post",
        &description,
        json!({
            "type": "object",
            "properties": {
                "text": { "type": "string", "minLength": 1, "maxLength": teeny_x::MAX_POST_LENGTH },
                "replyToPostId": { "type": "string", "pattern": "^\\d+$" },
                "quotePostId": { "type": "string", "pattern": "^\\d+$" },
                "gifId": {
                    "type": "string",
                    "pattern": "^[A-Za-z0-9]+$",
                    "description": "A GIF id returned by find_gif, attached as media."
                },
                "mediaId": {
                    "type": "string",
                    "pattern": "^(img|vid)_[a-z0-9]+_\\d+$",
                    "description": "A mediaId returned by generate_image or generate_video, attached as media."
                }
            },
            "required": ["text"],
            "additionalProperties": false
        }),
        Access::Write,
        move |args| {
            let tool = Arc::clone(&tool);
            async move { post(&tool, args).await }
        },
    )
}

async fn post(tool: &XPostTool, args: serde_json::Value) -> ToolResult {
    let args = match parse::<PostArgs>(args) {
        Ok(args) => args,
        Err(refused) => return refused,
    };
    if let Err(refused) = check_args(&args) {
        return refused;
    }
    if args.gif_id.is_some() && tool.gifs.is_none() {
        return answer("Refused: no GIF source is configured, so gifId cannot be attached.".into());
    }
    if args.gif_id.is_some() && args.media_id.is_some() {
        return answer(
            "Refused: a post carries either a GIF or a generated file, not both. Drop one.".into(),
        );
    }
    let generated: Option<StoredMedia> = match &args.media_id {
        Some(id) => match tool.media.as_ref().and_then(|media| media.get(id)) {
            Some(stored) => Some(stored),
            None => {
                return answer(format!(
                    "Refused: no generated media with id {id}. Use an id that generate_image or generate_video returned in this conversation."
                ));
            }
        },
        None => None,
    };

    let reply_to = args.reply_to_post_id.as_deref().map(PostId::from);
    let draft = Draft {
        reply_to: reply_to.clone(),
        quote: args.quote_post_id.as_deref().map(PostId::from),
        ..Draft::new(&args.text)
    };
    // Only a Giphy id or a file this process generated can be attached,
    // never an arbitrary URL, so the media this account posts is always
    // something it can account for.
    let attach = || async {
        if let Some(stored) = &generated {
            return upload(&tool.x, stored.data.clone(), x_media_type(stored.mime_type)).await;
        }
        let (Some(gif_id), Some(gifs)) = (&args.gif_id, &tool.gifs) else {
            return Ok(Vec::new());
        };
        let found = gifs.get(gif_id).await.map_err(|error| {
            PublishError::Refused(Refusal::new(format!(
                "the GIF could not be fetched ({error})."
            )))
        })?;
        let Some(gif) = found else {
            return Err(PublishError::Refused(Refusal::new(format!(
                "no GIF found for id {gif_id}. Call find_gif again and use an id it returned."
            ))));
        };
        let (data, mime) = gifs.download(&gif).await.map_err(|error| {
            PublishError::Refused(Refusal::new(format!(
                "the GIF could not be fetched ({error})."
            )))
        })?;
        upload(&tool.x, data, x_media_type(mime)).await
    };
    let published = match tool.publisher.post_with_media(draft, attach).await {
        Ok(published) => published,
        Err(error) => return refusal_of(error),
    };
    if let Some(pacing) = &tool.pacing {
        pacing.record(&published, reply_to.as_ref()).await;
    }
    if published.dry_run {
        let would = WouldHave {
            text: &published.text,
            reply_to_post_id: args.reply_to_post_id.as_deref(),
            quote_post_id: args.quote_post_id.as_deref(),
            gif_id: args.gif_id.as_deref(),
            media_id: args.media_id.as_deref(),
        };
        return ToolResult::success(format!(
            "DRY RUN, nothing was published. Would have posted: {}",
            serde_json::to_string(&would).unwrap_or_default()
        ));
    }
    let attached = match (&generated, &args.gif_id) {
        (Some(stored), _) => format!(
            " with a generated {}",
            match stored.kind {
                MediaKind::Image => "image",
                MediaKind::Video => "video",
            }
        ),
        (None, Some(_)) => " with a GIF".into(),
        (None, None) => String::new(),
    };
    ToolResult::success(format!("Published post {}{attached}.", published.id))
}

#[cfg(test)]
#[path = "post_tests.rs"]
mod tests;
