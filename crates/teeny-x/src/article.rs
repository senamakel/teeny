//! X Articles: a draft first, then published, which is what puts the post on
//! the timeline. Both calls sign as the account. X gates the endpoint on
//! Premium+ and answers 4xx otherwise; that answer comes back as
//! [`XError::ArticleRefused`] with its status, so the journal says why.
//!
//! The types are the Articles API's body format (Draft.js content state),
//! serialized exactly as X takes it, snake_case. Only the subset the article
//! pipeline emits is named here.

use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use teeny_core::types::PostId;

use crate::client::{Auth, XApi};
use crate::error::{XError, XResult};
use crate::oauth::percent_encode;
use crate::wire::Envelope;

/// An article, ready to publish.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArticleDraft {
    pub title: String,
    pub content_state: ContentState,
    /// A media id from `upload_media`, shown above the title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_media_id: Option<String>,
}

/// The body: text blocks in reading order, and the entities (images,
/// dividers, links) atomic blocks point at by index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContentState {
    pub blocks: Vec<ArticleBlock>,
    pub entities: Vec<ArticleEntity>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArticleBlock {
    pub text: String,
    #[serde(rename = "type")]
    pub kind: BlockType,
    /// Offsets and lengths in UTF-16 code units of `text`, as X counts them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inline_style_ranges: Vec<InlineStyle>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entity_ranges: Vec<EntityRange>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BlockType {
    Unstyled,
    HeaderOne,
    HeaderTwo,
    HeaderThree,
    UnorderedListItem,
    OrderedListItem,
    Blockquote,
    Atomic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InlineStyle {
    pub offset: u32,
    pub length: u32,
    pub style: Style,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Style {
    Bold,
    Italic,
    Strikethrough,
}

/// `key` is an index into the article's `entities`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityRange {
    pub key: u32,
    pub offset: u32,
    pub length: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArticleEntity {
    /// The index into `entities`, as a string; X keys entities that way.
    pub key: String,
    pub value: EntityValue,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntityValue {
    #[serde(rename = "type")]
    pub kind: EntityKind,
    pub mutability: Mutability,
    pub data: EntityData,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntityKind {
    Image,
    Divider,
    Link,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mutability {
    Immutable,
    Mutable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntityData {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub media_items: Vec<MediaItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaItem {
    pub media_category: String,
    pub media_id: String,
}

/// What publishing made: the article, and the post that carries it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedArticle {
    pub article_id: String,
    pub post_id: PostId,
}

#[derive(Debug, Deserialize)]
struct DraftData {
    id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PublishData {
    post_id: Option<PostId>,
}

impl XApi {
    /// Drafts and publishes an article. Crate-private: publishing goes
    /// through [`crate::XPublisher`], which honours `DRY_RUN`.
    pub(crate) async fn publish_article(&self, draft: &ArticleDraft) -> XResult<PublishedArticle> {
        self.draft_and_publish(draft)
            .await
            .map_err(|error| match error {
                XError::Http { status, body } => XError::ArticleRefused { status, body },
                other => other,
            })
    }

    async fn draft_and_publish(&self, draft: &ArticleDraft) -> XResult<PublishedArticle> {
        let mut body = Map::new();
        body.insert("title".into(), json!(draft.title));
        body.insert(
            "content_state".into(),
            serde_json::to_value(&draft.content_state)
                .map_err(|error| XError::Decode(error.to_string()))?,
        );
        if let Some(cover) = &draft.cover_media_id {
            body.insert(
                "cover_media".into(),
                json!({ "media_category": "tweet_image", "media_id": cover }),
            );
        }
        let created: Envelope<DraftData> = self
            .call(
                Method::POST,
                "/2/articles/draft",
                &[],
                Some(&Value::Object(body)),
                Auth::User,
            )
            .await?;
        let Some(article_id) = created.data.and_then(|data| data.id) else {
            return Err(XError::Missing(format!(
                "X did not return an article id: {}",
                created.errors.unwrap_or_default()
            )));
        };

        let published: Envelope<PublishData> = self
            .call(
                Method::POST,
                &format!("/2/articles/{}/publish", percent_encode(&article_id)),
                &[],
                None,
                Auth::User,
            )
            .await?;
        let Some(post_id) = published.data.and_then(|data| data.post_id) else {
            return Err(XError::Missing(format!(
                "X did not return a post for article {article_id}: {}",
                published.errors.unwrap_or_default()
            )));
        };
        Ok(PublishedArticle {
            article_id,
            post_id,
        })
    }
}

#[cfg(test)]
#[path = "article_tests.rs"]
mod tests;
