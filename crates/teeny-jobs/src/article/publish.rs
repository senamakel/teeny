//! A saved article, published as it is: the rehearsal wrote the record and
//! the pictures; this reads them back, uploads, sends through the cadence,
//! and marks the record published. Nothing is rewritten or redrawn, so what
//! was reviewed is what goes out.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde_json::json;
use teeny_agent::deps::Deps;
use teeny_cadence::cadence::PublishError as CadenceError;
use teeny_cadence::clock::iso;
use teeny_core::types::PostId;
use teeny_x::article::ArticleDraft;
use teeny_x::model::{Media, MediaType};
use teeny_x::{NoopGuard, XPublisher};

use super::content_state::{ImageRef, markdown_to_content_state, record_shape};
use super::store::{ArticleRecord, ArticleStatus, FileArticleStore};

pub async fn publish_saved(
    slug: &str,
    articles: &FileArticleStore,
    deps: &Deps,
    now: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<ArticleRecord> {
    let record = articles
        .get(slug)
        .await?
        .ok_or_else(|| anyhow::anyhow!("no article saved as {slug}"))?;
    anyhow::ensure!(
        record.status != ArticleStatus::Published,
        "{slug} was already published as post {}",
        record.post_id.as_deref().unwrap_or("?")
    );
    anyhow::ensure!(
        !record.markdown.trim().is_empty() && !record.title.is_empty(),
        "{slug} has no text to publish"
    );
    let upload = async |file: &str| -> anyhow::Result<String> {
        let (data, mime) = articles
            .media(slug, file)
            .await?
            .ok_or_else(|| anyhow::anyhow!("{slug} is missing its picture {file}"))?;
        let mime = match mime {
            "image/jpeg" => MediaType::Jpeg,
            "image/webp" => MediaType::Webp,
            _ => MediaType::Png,
        };
        Ok(deps.x.upload_media(&Media { data, mime }).await?.media_id)
    };
    let cover_media_id = match &record.cover {
        Some(cover) => Some(upload(&cover.file).await?),
        None => None,
    };
    let mut refs = BTreeMap::new();
    let mut images = Vec::new();
    for image in &record.images {
        let media_id = upload(&image.file).await?;
        refs.insert(
            image.key.clone(),
            ImageRef {
                media_id: media_id.clone(),
                caption: Some(image.caption.clone()),
            },
        );
        images.push(super::store::ArticleImage {
            media_id: Some(media_id),
            ..image.clone()
        });
    }
    let state = markdown_to_content_state(&record.markdown, &refs);
    let mut draft = ArticleRecord {
        status: ArticleStatus::Draft,
        cover: record
            .cover
            .clone()
            .map(|cover| super::store::ArticleImage {
                media_id: cover_media_id.clone(),
                ..cover
            }),
        images,
        content_state: Some(record_shape(&state)),
        ..record.clone()
    };
    articles.save(&draft, &[]).await?;
    let publisher = XPublisher::new(
        Arc::clone(&deps.x),
        Arc::new(NoopGuard),
        deps.settings.dry_run,
    );
    let article = ArticleDraft {
        title: record.title.clone(),
        content_state: state,
        cover_media_id,
    };
    let article_id = Mutex::new(String::new());
    let published = deps
        .cadence
        .publish_own(
            "article",
            Some(&record.title),
            async || -> anyhow::Result<PostId> {
                let published = publisher.publish_article(&article).await?;
                *article_id.lock().unwrap_or_else(|p| p.into_inner()) = published.article_id;
                Ok(published.post_id)
            },
        )
        .await;
    let post_id = match published {
        Ok(post_id) => post_id,
        Err(error) => {
            let message = match error {
                CadenceError::Held(held) => held.to_string(),
                CadenceError::Ledger(error) => format!("cadence: {error}"),
                CadenceError::Publish(error) => error.to_string(),
            };
            draft.status = ArticleStatus::Failed;
            draft.error = Some(message.clone());
            articles.save(&draft, &[]).await?;
            anyhow::bail!(message);
        }
    };
    draft.status = ArticleStatus::Published;
    draft.x_article_id = Some(article_id.into_inner().unwrap_or_default());
    draft.post_id = Some(post_id.to_string());
    draft.published_at = Some(iso(now));
    articles.save(&draft, &[]).await?;
    tracing::info!(slug, post = %post_id, "article: published");
    let payload = json!({
        "slug": slug,
        "title": record.title,
        "dek": record.dek,
        "engine": record.engine,
        "premise": record.thought.as_ref().map(|t| t.premise.clone()),
        "sections": record.sections.iter().map(|s| s.heading.clone()).collect::<Vec<_>>(),
        "published": true,
    });
    deps.memory
        .capture_activity("article", post_id.as_str(), payload, None)
        .await
        .ok();
    Ok(draft)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::article::store::ArticleImage;
    use crate::backrooms::testing::{FixedMemory, scratch};
    use crate::narrative::pipeline::tests::{live_deps, x_server};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, ResponseTemplate};

    #[tokio::test]
    async fn uploads_the_saved_pictures_sends_the_saved_text_and_marks_the_record_published_once() {
        let server = x_server().await;
        Mock::given(method("POST"))
            .and(path("/2/articles/draft"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({ "data": { "id": "a1" } })),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/2/articles/a1/publish"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({ "data": { "post_id": "9001" } })),
            )
            .mount(&server)
            .await;
        let deps = live_deps(&server, Arc::new(FixedMemory::default()), false);
        let dir = scratch("publish-saved");
        let store = FileArticleStore::new(&dir);
        let image = |key: &str| ArticleImage {
            key: key.into(),
            file: format!("{key}.png"),
            mime_type: "image/png".into(),
            prompt: "p".into(),
            caption: "c".into(),
            media_id: None,
        };
        let record = ArticleRecord {
            slug: "20260921-the-inch".into(),
            title: "the inch".into(),
            dek: "a door".into(),
            status: ArticleStatus::DryRun,
            markdown: "_a door_\n\n## the gap\n\n![c](image:s1)\n\nbody.".into(),
            cover: Some(image("cover")),
            images: vec![image("s1")],
            ..ArticleRecord::new("2026-09-21T12:00:00.000Z".into())
        };
        store
            .save(
                &record,
                &[("cover.png".into(), vec![1]), ("s1.png".into(), vec![2])],
            )
            .await
            .unwrap();
        let now = chrono::DateTime::parse_from_rfc3339("2026-09-22T19:00:00Z")
            .unwrap()
            .to_utc();
        let out = publish_saved("20260921-the-inch", &store, &deps, now)
            .await
            .unwrap();
        assert_eq!(out.status, ArticleStatus::Published);
        assert_eq!(out.post_id.as_deref(), Some("9001"));
        assert_eq!(out.images[0].media_id.as_deref(), Some("m1"));
        assert_eq!(
            store
                .get("20260921-the-inch")
                .await
                .unwrap()
                .unwrap()
                .status,
            ArticleStatus::Published
        );
        let again = publish_saved("20260921-the-inch", &store, &deps, now)
            .await
            .unwrap_err();
        assert!(again.to_string().contains("already published as post 9001"));
        std::fs::remove_dir_all(dir).ok();
    }
}
