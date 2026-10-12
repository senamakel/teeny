use chrono::TimeZone;

use super::*;
use crate::article::stages::tests::structure;
use crate::backrooms::testing::scratch;

fn record(slug: &str, status: ArticleStatus, created_at: &str) -> ArticleRecord {
    ArticleRecord {
        slug: slug.into(),
        title: "the inch".into(),
        dek: "a door".into(),
        status,
        ..ArticleRecord::new(created_at.into())
    }
}

#[tokio::test]
async fn slugs_a_title_by_the_day_and_finds_a_free_one() {
    let at = Utc.with_ymd_and_hms(2026, 9, 21, 23, 0, 0).unwrap();
    assert_eq!(
        slug_for(at, "The Inch: a door, a ruler & a ledger"),
        "20260921-the-inch-a-door-a-ruler-a-ledger"
    );
    assert_eq!(slug_for(at, "!!!"), "20260921");
    let dir = scratch("article-slugs");
    let store = FileArticleStore::new(&dir);
    store
        .save(
            &record(
                "20260921-the-inch",
                ArticleStatus::DryRun,
                "2026-09-21T12:00:00.000Z",
            ),
            &[],
        )
        .await
        .unwrap();
    assert_eq!(
        free_slug(&store, "20260921-the-inch").await.unwrap(),
        "20260921-the-inch-2"
    );
    assert_eq!(
        free_slug(&store, "20260921-other").await.unwrap(),
        "20260921-other"
    );
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn writes_the_record_and_the_pictures_in_the_shape_the_server_reads() {
    let dir = scratch("articles");
    let store = FileArticleStore::new(dir.join("articles"));
    assert!(store.records().await.unwrap().is_empty());
    let png = vec![0x89, 0x50, 0x4e, 0x47];
    let mut first = record(
        "20260921-the-inch",
        ArticleStatus::DryRun,
        "2026-09-21T12:00:00.000Z",
    );
    first.cover = Some(ArticleImage {
        key: "cover".into(),
        file: "cover.png".into(),
        mime_type: "image/png".into(),
        prompt: "p".into(),
        caption: String::new(),
        media_id: None,
    });
    first.word_count = 3;
    first.engine = Some("the box".into());
    first.structure = Some(structure());
    store
        .save(&first, &[("cover.png".into(), png.clone())])
        .await
        .unwrap();
    let mut later = record(
        "20260924-later",
        ArticleStatus::Published,
        "2026-09-24T22:00:00.000Z",
    );
    later.published_at = Some("2026-09-24T23:00:00.000Z".into());
    later.post_id = Some("9".into());
    store.save(&later, &[]).await.unwrap();

    let raw: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("articles/20260921-the-inch/article.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(raw["status"], "dry-run");
    assert_eq!(raw["createdAt"], "2026-09-21T12:00:00.000Z");
    assert_eq!(raw["wordCount"], 3);
    assert_eq!(raw["cover"]["mimeType"], "image/png");
    assert_eq!(raw["structure"]["sections"][0]["heading"], "the gap");

    assert_eq!(
        store
            .records()
            .await
            .unwrap()
            .iter()
            .map(|r| r.slug.as_str())
            .collect::<Vec<_>>(),
        ["20260924-later", "20260921-the-inch"]
    );
    assert_eq!(
        store
            .get("20260921-the-inch")
            .await
            .unwrap()
            .unwrap()
            .structure
            .unwrap()
            .title,
        "the inch"
    );
    assert!(store.get("../etc").await.unwrap().is_none());
    assert_eq!(
        store.media("20260921-the-inch", "cover.png").await.unwrap(),
        Some((png.clone(), "image/png"))
    );
    assert!(
        store
            .media("20260921-the-inch", "../article.json")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .media("20260921-the-inch", "nope.png")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        store.latest_published_at().await.unwrap().as_deref(),
        Some("2026-09-24T23:00:00.000Z")
    );
    assert_eq!(
        store.last_engine().await.unwrap().as_deref(),
        Some("the box")
    );
    let refused = store
        .save(&first, &[("evil.sh".into(), png)])
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("not an image"));
    std::fs::remove_dir_all(dir).ok();
}

// legacy article store: "finds a published article by the post that carried it"
#[tokio::test]
async fn looks_up_a_published_article_by_its_post_for_the_reply_job() {
    use crate::reply::ArticleLookup;

    let dir = scratch("article-lookup");
    let store = FileArticleStore::new(&dir);
    let published = ArticleRecord {
        post_id: Some("42".into()),
        published_at: Some("2026-09-22T19:00:00.000Z".into()),
        markdown: "# the inch\n\nbody".into(),
        ..record(
            "20260922-the-inch",
            ArticleStatus::Published,
            "2026-09-22T12:00:00.000Z",
        )
    };
    let rehearsed = ArticleRecord {
        post_id: Some("43".into()),
        ..record(
            "20260921-a-rehearsal",
            ArticleStatus::DryRun,
            "2026-09-21T12:00:00.000Z",
        )
    };
    store.save(&published, &[]).await.unwrap();
    store.save(&rehearsed, &[]).await.unwrap();

    let found = store.find_by_post_id("42").await.unwrap().unwrap();
    assert_eq!(found.slug, "20260922-the-inch");
    assert_eq!(found.title, "the inch");
    assert_eq!(found.dek, "a door");
    assert_eq!(found.markdown, "# the inch\n\nbody");
    assert_eq!(found.post_id.as_deref(), Some("42"));
    assert_eq!(
        found.published_at.as_deref(),
        Some("2026-09-22T19:00:00.000Z")
    );
    // Only a published one is the account's article.
    assert_eq!(store.find_by_post_id("43").await.unwrap(), None);
    assert_eq!(store.find_by_post_id("").await.unwrap(), None);
    std::fs::remove_dir_all(dir).ok();
}
