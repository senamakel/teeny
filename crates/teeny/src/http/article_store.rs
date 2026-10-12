//! The articles on disk, read side: one directory per article under
//! `ARTICLES_DIR`, `article.json` and the pictures beside it. A port of the
//! reading half of `legacy/src/article/store.ts`; the pipeline that writes
//! them is a later milestone.

use std::io::ErrorKind;
use std::path::PathBuf;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Where an article got to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ArticleStatus {
    Structure,
    Draft,
    DryRun,
    Held,
    Skipped,
    Published,
    Failed,
}

/// A picture beside the record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArticleImage {
    /// `cover` or the section key.
    pub key: String,
    pub file: String,
    pub mime_type: ImageType,
    pub prompt: String,
    #[serde(default)]
    pub caption: String,
    /// X's media id once uploaded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageType {
    #[serde(rename = "image/png")]
    Png,
    #[serde(rename = "image/jpeg")]
    Jpeg,
    #[serde(rename = "image/webp")]
    Webp,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArticleSection {
    pub key: String,
    pub heading: String,
    pub markdown: String,
}

/// `article.json`. The stages' own outputs (the room, the plan, the proof)
/// are kept as they were written; only what the site and the store read is
/// typed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArticleRecord {
    pub slug: String,
    pub title: String,
    pub dek: String,
    pub status: ArticleStatus,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thought: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judge: Option<Value>,
    #[serde(default)]
    pub rooms: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structure: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structure_check: Option<Value>,
    #[serde(default)]
    pub sections: Vec<ArticleSection>,
    #[serde(default)]
    pub markdown: String,
    /// A number as written: an integer stays an integer on the way out.
    #[serde(default = "zero")]
    pub word_count: serde_json::Number,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proof: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub critique: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover: Option<ArticleImage>,
    #[serde(default)]
    pub images: Vec<ArticleImage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_state: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x_article_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub problems: Vec<String>,
}

fn zero() -> serde_json::Number {
    0.into()
}

/// One line of `/articles`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArticleSummary {
    pub slug: String,
    pub title: String,
    pub dek: String,
    pub status: ArticleStatus,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post_id: Option<String>,
    /// The cover's file name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cover: Option<String>,
    pub word_count: serde_json::Number,
}

/// A picture's bytes and type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Media {
    pub data: Vec<u8>,
    pub mime_type: &'static str,
}

/// The read side of the store.
#[async_trait]
pub trait ArticleStore: Send + Sync {
    /// Published articles, newest first; everything with `all`.
    async fn list(&self, all: bool, limit: usize) -> anyhow::Result<Vec<ArticleSummary>>;
    async fn get(&self, slug: &str) -> anyhow::Result<Option<ArticleRecord>>;
    async fn media(&self, slug: &str, file: &str) -> anyhow::Result<Option<Media>>;
}

/// The store on disk.
pub struct FileArticleStore {
    dir: PathBuf,
}

impl FileArticleStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }
}

impl FileArticleStore {
    /// Every record, newest first by creation.
    async fn records(&self) -> anyhow::Result<Vec<ArticleRecord>> {
        let mut entries = match tokio::fs::read_dir(&self.dir).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut records = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if !entry.file_type().await?.is_dir() || !is_slug(&name) {
                continue;
            }
            if let Some(record) = self.get(&name).await? {
                records.push(record);
            }
        }
        records.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(records)
    }
}

#[async_trait]
impl ArticleStore for FileArticleStore {
    async fn list(&self, all: bool, limit: usize) -> anyhow::Result<Vec<ArticleSummary>> {
        Ok(self
            .records()
            .await?
            .iter()
            .filter(|record| all || record.status == ArticleStatus::Published)
            .take(limit)
            .map(summarise)
            .collect())
    }

    /// `None` for a bad slug, a missing record, or one that does not
    /// validate; an unreadable or unparseable file is an error, as it was.
    async fn get(&self, slug: &str) -> anyhow::Result<Option<ArticleRecord>> {
        if !is_slug(slug) {
            return Ok(None);
        }
        let bytes = match tokio::fs::read(self.dir.join(slug).join("article.json")).await {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let json: Value = serde_json::from_slice(&bytes)?;
        Ok(serde_json::from_value::<ArticleRecord>(json)
            .ok()
            .filter(ArticleRecord::is_valid))
    }

    async fn media(&self, slug: &str, file: &str) -> anyhow::Result<Option<Media>> {
        if !is_slug(slug) || !is_image_file(file) {
            return Ok(None);
        }
        match tokio::fs::read(self.dir.join(slug).join(file)).await {
            Ok(data) => Ok(Some(Media {
                data,
                mime_type: mime_type(file),
            })),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }
}

impl ArticleRecord {
    /// What serde cannot check: the slug and the picture names.
    fn is_valid(&self) -> bool {
        is_slug(&self.slug)
            && self
                .cover
                .iter()
                .chain(&self.images)
                .all(|image| !image.key.is_empty() && is_image_file(&image.file))
    }
}

pub fn summarise(record: &ArticleRecord) -> ArticleSummary {
    let set = |value: &Option<String>| value.clone().filter(|value| !value.is_empty());
    ArticleSummary {
        slug: record.slug.clone(),
        title: record.title.clone(),
        dek: record.dek.clone(),
        status: record.status,
        created_at: record.created_at.clone(),
        published_at: set(&record.published_at),
        post_id: set(&record.post_id),
        cover: record.cover.as_ref().map(|cover| cover.file.clone()),
        word_count: record.word_count.clone(),
    }
}

/// `^[a-z0-9-]+$`: no dots and no slashes, so never a path out.
pub fn is_slug(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

/// `^[a-z0-9-]+\.(png|jpg|webp)$`.
pub fn is_image_file(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(stem, extension)| {
        is_slug(stem) && matches!(extension, "png" | "jpg" | "webp")
    })
}

fn mime_type(file: &str) -> &'static str {
    match file.rsplit_once('.').map(|(_, extension)| extension) {
        Some("png") => "image/png",
        Some("jpg") => "image/jpeg",
        Some("webp") => "image/webp",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
#[path = "article_store_tests.rs"]
mod tests;
