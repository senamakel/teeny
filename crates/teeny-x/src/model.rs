//! What the client returns and takes: plain, serde-serializable values, so a
//! tool is a thin wrapper that hands them to a model, and the cache stores
//! them as they are.
//!
//! The JSON shape (camelCase, absent fields omitted) is the legacy `XPost`'s,
//! so rows the TypeScript cache wrote read back unchanged.

use serde::{Deserialize, Serialize};
use teeny_core::types::{PostId, UserId};

/// One post, as far as the fetch asked for it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct XPost {
    pub id: PostId,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_id: Option<UserId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<PostId>,
    /// ISO 8601, as X sends it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    /// Absent when the post is not a reply; present, the parent's author.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_reply_to_user_id: Option<UserId>,
    /// Pictures and clips attached, when the fetch asked for them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub media: Vec<XPostMedia>,
    /// The public counters, when the fetch asked for them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metrics: Option<XPostMetrics>,
}

impl XPost {
    /// A post with only its id and text known.
    pub fn new(id: impl Into<PostId>, text: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
            author_id: None,
            author_username: None,
            conversation_id: None,
            created_at: None,
            in_reply_to_user_id: None,
            media: Vec::new(),
            metrics: None,
        }
    }
}

/// One attachment. `url` is set for a photo; a video or an animated GIF only
/// exposes a preview frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct XPostMedia {
    /// `photo`, `video`, `animated_gif`, or whatever X adds next.
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alt_text: Option<String>,
}

/// X's public counters on a post. Impressions are the author's to see, so on
/// somebody else's post they are absent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct XPostMetrics {
    pub id: PostId,
    pub likes: u64,
    pub replies: u64,
    pub reposts: u64,
    pub quotes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub impressions: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bookmarks: Option<u64>,
}

/// The counters of one of the account's own posts, read through the
/// account's own credentials, which is why impressions are always there.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OwnPostMetrics {
    pub id: PostId,
    pub impressions: u64,
    pub likes: u64,
    pub replies: u64,
    pub reposts: u64,
    pub quotes: u64,
}

/// One trend: the phrase, and how many posts carry it when X says.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Trend {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub posts: Option<u64>,
}

/// Which trends to ask for: worldwide (WOEID 1) and 20 unless told.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrendOptions {
    pub woeid: Option<u32>,
    pub max: Option<u32>,
}

/// The account the credentials act as, read from the token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub id: UserId,
    pub username: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub followers: Option<u64>,
}

/// How a recent search orders its results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortOrder {
    Recency,
    Relevancy,
}

impl SortOrder {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Recency => "recency",
            Self::Relevancy => "relevancy",
        }
    }
}

/// The knobs on a recent search. Times are ISO 8601, within the last 7 days.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchOptions {
    /// Clamped to X's 10..=100; asking for fewer still bills for 10.
    pub max_results: Option<u32>,
    pub since_id: Option<PostId>,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
    pub sort_order: Option<SortOrder>,
    /// A `next_token` from an earlier [`SearchPage`].
    pub pagination_token: Option<String>,
}

/// One page of a recent search.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchPage {
    pub posts: Vec<XPost>,
    /// Present when X has another page; pass it back as `pagination_token`.
    pub next_token: Option<String>,
    /// The newest id in the page, the next poll's `since_id`.
    pub newest_id: Option<PostId>,
}

/// Where a mention search starts. X refuses `since_id` and `start_time`
/// together, so a window is one or the other, never both.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum WindowStart {
    /// The whole 7 days X keeps.
    #[default]
    Anything,
    SinceId(PostId),
    StartTime(String),
}

/// The mention poll's window.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MentionWindow {
    pub max_results: Option<u32>,
    pub start: WindowStart,
}

/// What a new post is attached to.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateOptions {
    pub reply_to: Option<PostId>,
    pub quote: Option<PostId>,
    #[serde(default)]
    pub media_ids: Vec<String>,
}

/// A post X accepted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Created {
    pub id: PostId,
}

/// The media types the account uploads. X sorts uploads into categories, not
/// by MIME type, and rejects the post (not the upload) when one lands in the
/// wrong pipeline, so the category is tied to the type here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaType {
    #[serde(rename = "image/gif")]
    Gif,
    #[serde(rename = "video/mp4")]
    Mp4,
    #[serde(rename = "image/png")]
    Png,
    #[serde(rename = "image/jpeg")]
    Jpeg,
    #[serde(rename = "image/webp")]
    Webp,
}

impl MediaType {
    pub fn mime(self) -> &'static str {
        match self {
            Self::Gif => "image/gif",
            Self::Mp4 => "video/mp4",
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Webp => "image/webp",
        }
    }

    /// `tweet_gif` for an animation, `tweet_image` for a still,
    /// `tweet_video` for a clip.
    pub fn category(self) -> &'static str {
        match self {
            Self::Gif => "tweet_gif",
            Self::Png | Self::Jpeg | Self::Webp => "tweet_image",
            Self::Mp4 => "tweet_video",
        }
    }
}

/// Bytes to upload. Fitting a still under X's image limit is the caller's
/// job (teeny-media), as it was `fitImageForX`'s.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Media {
    pub data: Vec<u8>,
    pub mime: MediaType,
}

/// An uploaded attachment, ready to put on a post.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Uploaded {
    pub media_id: String,
}
