//! Where articles live: one directory per article under `ARTICLES_DIR`,
//! `article.json` and the pictures beside it, exactly as the server's
//! `/articles` reads them (`crates/teeny/src/http/article_store.rs`): the
//! same camelCase keys, the kebab-case status, the image MIME strings, an
//! integer `wordCount`. The record carries every stage's output so a
//! rehearsal can be read file by file.

use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::LazyLock;

use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::proofread::Proof;
use super::stages::{Flow, Structure, StructureCheck};
use crate::backrooms::Seed;
use crate::backrooms::distill::Thought;
use crate::backrooms::evaluate::Craziness;
use crate::backrooms::store::slugify;

pub static SLUG: LazyLock<Regex> = LazyLock::new(|| Regex::new("^[a-z0-9-]+$").expect("valid"));
pub static IMAGE_FILE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z0-9-]+\.(png|jpg|webp)$").expect("valid"));

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArticleImage {
    /// `cover` or the section key.
    pub key: String,
    pub file: String,
    /// `image/png`, `image/jpeg`, `image/webp`.
    pub mime_type: String,
    pub prompt: String,
    #[serde(default)]
    pub caption: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomLine {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rejected: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SectionRecord {
    pub key: String,
    pub heading: String,
    pub markdown: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowRecord {
    pub review: Flow,
    pub applied: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refused: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProofRecord {
    pub review: Proof,
    pub applied: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refused: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<Proof>,
}

/// `article.json`.
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
    pub seed: Option<Seed>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thought: Option<Thought>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judge: Option<Craziness>,
    #[serde(default)]
    pub rooms: Vec<RoomLine>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structure: Option<Structure>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structure_check: Option<StructureCheck>,
    #[serde(default)]
    pub sections: Vec<SectionRecord>,
    #[serde(default)]
    pub markdown: String,
    #[serde(default)]
    pub word_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow: Option<FlowRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proof: Option<ProofRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub critique: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover: Option<ArticleImage>,
    #[serde(default)]
    pub images: Vec<ArticleImage>,
    /// The body as X takes it, camelCase as the legacy kept it.
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

impl ArticleRecord {
    /// A record that has only begun.
    pub fn new(created_at: String) -> Self {
        let digits: String = created_at
            .chars()
            .filter(char::is_ascii_digit)
            .take(14)
            .collect();
        Self {
            slug: format!("pending-{digits}"),
            title: String::new(),
            dek: String::new(),
            status: ArticleStatus::Structure,
            created_at,
            published_at: None,
            engine: None,
            seed: None,
            run_id: None,
            run_path: None,
            thought: None,
            judge: None,
            rooms: Vec::new(),
            structure: None,
            structure_check: None,
            sections: Vec::new(),
            markdown: String::new(),
            word_count: 0,
            flow: None,
            proof: None,
            critique: None,
            cover: None,
            images: Vec::new(),
            content_state: None,
            x_article_id: None,
            post_id: None,
            error: None,
            problems: Vec::new(),
        }
    }
}

/// The articles on disk.
#[derive(Debug, Clone)]
pub struct FileArticleStore {
    pub dir: PathBuf,
}

impl FileArticleStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub async fn get(&self, slug: &str) -> anyhow::Result<Option<ArticleRecord>> {
        if !SLUG.is_match(slug) {
            return Ok(None);
        }
        match tokio::fs::read(self.dir.join(slug).join("article.json")).await {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes).ok()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// Writes the record, and any pictures given, under the article's directory.
    pub async fn save(
        &self,
        record: &ArticleRecord,
        files: &[(String, Vec<u8>)],
    ) -> anyhow::Result<()> {
        anyhow::ensure!(SLUG.is_match(&record.slug), "not a slug: {}", record.slug);
        let dir = self.dir.join(&record.slug);
        tokio::fs::create_dir_all(&dir).await?;
        for (file, data) in files {
            anyhow::ensure!(IMAGE_FILE.is_match(file), "not an image file name: {file}");
            tokio::fs::write(dir.join(file), data).await?;
        }
        let path = dir.join("article.json");
        let temporary = dir.join("article.json.tmp");
        tokio::fs::write(
            &temporary,
            format!("{}\n", serde_json::to_string_pretty(record)?),
        )
        .await?;
        tokio::fs::rename(&temporary, &path).await?;
        Ok(())
    }

    pub async fn media(
        &self,
        slug: &str,
        file: &str,
    ) -> anyhow::Result<Option<(Vec<u8>, &'static str)>> {
        if !SLUG.is_match(slug) || !IMAGE_FILE.is_match(file) {
            return Ok(None);
        }
        match tokio::fs::read(self.dir.join(slug).join(file)).await {
            Ok(data) => Ok(Some((data, mime_of(file)))),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// Every record, newest first by creation.
    pub async fn records(&self) -> anyhow::Result<Vec<ArticleRecord>> {
        let mut entries = match tokio::fs::read_dir(&self.dir).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut records = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            if let Some(name) = entry.file_name().to_str()
                && entry.file_type().await?.is_dir()
                && let Some(record) = self.get(name).await?
            {
                records.push(record);
            }
        }
        records.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(records)
    }

    pub async fn latest_published_at(&self) -> anyhow::Result<Option<String>> {
        Ok(self
            .records()
            .await?
            .into_iter()
            .find(|r| r.status == ArticleStatus::Published && r.published_at.is_some())
            .and_then(|r| r.published_at))
    }

    /// The engine of the newest article that got as far as a structure.
    pub async fn last_engine(&self) -> anyhow::Result<Option<String>> {
        Ok(self
            .records()
            .await?
            .into_iter()
            .find(|r| r.structure.is_some())
            .and_then(|r| r.engine))
    }
}

/// The reply job reads an article under the post that carried it: only a
/// published one, as the legacy store's `findByPostId`.
#[async_trait::async_trait]
impl crate::reply::ArticleLookup for FileArticleStore {
    async fn find_by_post_id(
        &self,
        post_id: &str,
    ) -> anyhow::Result<Option<crate::reply::ReplyArticle>> {
        if post_id.is_empty() {
            return Ok(None);
        }
        Ok(self
            .records()
            .await?
            .into_iter()
            .find(|record| {
                record.status == ArticleStatus::Published
                    && record.post_id.as_deref() == Some(post_id)
            })
            .map(|record| crate::reply::ReplyArticle {
                slug: record.slug,
                title: record.title,
                dek: record.dek,
                markdown: record.markdown,
                published_at: record.published_at,
                post_id: record.post_id,
            }))
    }
}

pub fn mime_of(file: &str) -> &'static str {
    match file.rsplit('.').next() {
        Some("png") => "image/png",
        Some("jpg") => "image/jpeg",
        Some("webp") => "image/webp",
        _ => "application/octet-stream",
    }
}

/// `YYYYMMDD-<title>`.
pub fn slug_for(now: DateTime<Utc>, title: &str) -> String {
    let stamp = now.format("%Y%m%d").to_string();
    let clean = slugify(title, 48).trim_end_matches('-').to_owned();
    if clean.is_empty() {
        stamp
    } else {
        format!("{stamp}-{clean}")
    }
}

/// The slug, with a `-2`, `-3`… when the store already has it.
pub async fn free_slug(store: &FileArticleStore, slug: &str) -> anyhow::Result<String> {
    if store.get(slug).await?.is_none() {
        return Ok(slug.to_owned());
    }
    for n in 2..100 {
        let candidate = format!("{slug}-{n}");
        if store.get(&candidate).await?.is_none() {
            return Ok(candidate);
        }
    }
    anyhow::bail!("no free slug for {slug}")
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
