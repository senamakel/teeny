//! X v2's JSON as it arrives (snake_case, everything optional), and the step
//! from it to [`crate::model`].

use std::collections::HashMap;

use serde::Deserialize;
use teeny_core::types::{PostId, UserId};

use crate::model::{XPost, XPostMedia, XPostMetrics};

/// Every v2 response: `data`, the expansions in `includes`, partial `errors`
/// and the paging `meta`.
#[derive(Debug, Deserialize)]
pub struct Envelope<T> {
    pub data: Option<T>,
    pub includes: Option<Includes>,
    #[serde(default)]
    pub errors: Option<serde_json::Value>,
    pub meta: Option<Meta>,
}

#[derive(Debug, Default, Deserialize)]
pub struct Meta {
    pub next_token: Option<String>,
    pub newest_id: Option<PostId>,
}

#[derive(Debug, Default, Deserialize)]
pub struct Includes {
    #[serde(default)]
    pub users: Vec<RawUser>,
    #[serde(default)]
    pub media: Vec<RawMedia>,
}

#[derive(Debug, Default, Deserialize)]
pub struct RawPost {
    pub id: Option<PostId>,
    pub text: Option<String>,
    pub author_id: Option<UserId>,
    pub conversation_id: Option<PostId>,
    pub created_at: Option<String>,
    pub in_reply_to_user_id: Option<UserId>,
    pub attachments: Option<RawAttachments>,
    pub public_metrics: Option<RawMetrics>,
}

#[derive(Debug, Default, Deserialize)]
pub struct RawAttachments {
    #[serde(default)]
    pub media_keys: Vec<String>,
}

/// X has called a repost both things; either spelling is read.
#[derive(Debug, Default, Deserialize)]
pub struct RawMetrics {
    pub like_count: Option<u64>,
    pub reply_count: Option<u64>,
    #[serde(alias = "retweet_count")]
    pub repost_count: Option<u64>,
    pub quote_count: Option<u64>,
    pub bookmark_count: Option<u64>,
    pub impression_count: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
pub struct RawUser {
    pub id: Option<UserId>,
    pub username: Option<String>,
    pub public_metrics: Option<RawUserMetrics>,
    pub connection_status: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
pub struct RawUserMetrics {
    pub followers_count: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
pub struct RawMedia {
    pub media_key: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub url: Option<String>,
    pub preview_image_url: Option<String>,
    pub alt_text: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RawTrend {
    pub trend_name: Option<String>,
    pub tweet_count: Option<u64>,
}

/// Posts X returned, with authors and attachments joined in from `includes`.
/// A post without an id or text is dropped.
pub fn to_posts(data: Vec<RawPost>, includes: Option<&Includes>) -> Vec<XPost> {
    let usernames: HashMap<&UserId, &str> = includes
        .map(|includes| {
            includes
                .users
                .iter()
                .filter_map(|user| Some((user.id.as_ref()?, user.username.as_deref()?)))
                .collect()
        })
        .unwrap_or_default();
    let attachments: HashMap<&str, &RawMedia> = includes
        .map(|includes| {
            includes
                .media
                .iter()
                .filter_map(|item| Some((item.media_key.as_deref()?, item)))
                .collect()
        })
        .unwrap_or_default();

    data.into_iter()
        .filter_map(|post| {
            let (Some(id), Some(text)) = (post.id, post.text) else {
                return None;
            };
            if text.is_empty() {
                return None;
            }
            let author_username = post
                .author_id
                .as_ref()
                .and_then(|author| usernames.get(author))
                .map(|name| (*name).to_owned());
            let media = post
                .attachments
                .map(|attached| attached.media_keys)
                .unwrap_or_default()
                .iter()
                .filter_map(|key| {
                    let item = attachments.get(key.as_str())?;
                    Some(XPostMedia {
                        kind: item.kind.clone()?,
                        url: item.url.clone(),
                        preview_url: item.preview_image_url.clone(),
                        alt_text: item.alt_text.clone(),
                    })
                })
                .collect();
            let metrics = post.public_metrics.map(|raw| to_metrics(&id, raw));
            Some(XPost {
                id,
                text,
                author_id: post.author_id,
                author_username,
                conversation_id: post.conversation_id,
                created_at: post.created_at,
                in_reply_to_user_id: post.in_reply_to_user_id,
                media,
                metrics,
            })
        })
        .collect()
}

fn to_metrics(id: &PostId, raw: RawMetrics) -> XPostMetrics {
    XPostMetrics {
        id: id.clone(),
        likes: raw.like_count.unwrap_or(0),
        replies: raw.reply_count.unwrap_or(0),
        reposts: raw.repost_count.unwrap_or(0),
        quotes: raw.quote_count.unwrap_or(0),
        impressions: raw.impression_count,
        bookmarks: raw.bookmark_count,
    }
}

#[cfg(test)]
#[path = "wire_tests.rs"]
mod tests;
