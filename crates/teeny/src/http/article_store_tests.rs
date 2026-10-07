use serde_json::{Value, json};

use super::*;
use crate::http::test_support::Scratch;

pub fn record(slug: &str, status: &str, created_at: &str) -> Value {
    json!({ "slug": slug, "title": slug, "dek": "d", "status": status, "createdAt": created_at })
}

pub fn save(dir: &Scratch, record: &Value) {
    let slug = record["slug"].as_str().unwrap();
    dir.write(&format!("{slug}/article.json"), serde_json::to_vec_pretty(record).unwrap());
}

#[tokio::test]
async fn lists_the_published_newest_first_and_everything_on_request() {
    let dir = Scratch::new("articles-list");
    save(&dir, &record("older", "published", "2026-09-01T00:00:00.000Z"));
    save(&dir, &record("newer", "published", "2026-09-02T00:00:00.000Z"));
    save(&dir, &record("rehearsal", "dry-run", "2026-09-03T00:00:00.000Z"));
    let store = FileArticleStore::new(&dir.0);

    let published: Vec<String> = store.list(false, 50).await.unwrap().into_iter().map(|a| a.slug).collect();
    assert_eq!(published, ["newer", "older"]);
    let all: Vec<String> = store.list(true, 50).await.unwrap().into_iter().map(|a| a.slug).collect();
    assert_eq!(all, ["rehearsal", "newer", "older"]);
    assert_eq!(store.list(true, 1).await.unwrap().len(), 1);
}

#[tokio::test]
async fn summarises_with_the_cover_file_and_only_the_fields_that_are_set() {
    let dir = Scratch::new("articles-summary");
    let mut full = record("the-inch", "published", "2026-09-23T23:00:00.000Z");
    full["publishedAt"] = json!("2026-09-23T23:05:00.000Z");
    full["postId"] = json!("9001");
    full["wordCount"] = json!(3);
    full["cover"] = json!({ "key": "cover", "file": "cover.png", "mimeType": "image/png", "prompt": "p" });
    save(&dir, &full);
    save(&dir, &record("bare", "published", "2026-09-01T00:00:00.000Z"));
    let store = FileArticleStore::new(&dir.0);

    let list = serde_json::to_value(store.list(false, 50).await.unwrap()).unwrap();
    assert_eq!(
        list,
        json!([
            {
                "slug": "the-inch", "title": "the-inch", "dek": "d", "status": "published",
                "createdAt": "2026-09-23T23:00:00.000Z", "publishedAt": "2026-09-23T23:05:00.000Z",
                "postId": "9001", "cover": "cover.png", "wordCount": 3,
            },
            {
                "slug": "bare", "title": "bare", "dek": "d", "status": "published",
                "createdAt": "2026-09-01T00:00:00.000Z", "wordCount": 0,
            },
        ])
    );
}

#[tokio::test]
async fn reads_a_record_with_its_defaults_and_skips_one_that_does_not_validate() {
    let dir = Scratch::new("articles-get");
    save(&dir, &record("good", "draft", "2026-09-01T00:00:00.000Z"));
    save(&dir, &record("bad-status", "pondering", "2026-09-01T00:00:00.000Z"));
    let mut bad_image = record("bad-image", "published", "2026-09-01T00:00:00.000Z");
    bad_image["cover"] = json!({ "key": "cover", "file": "../cover.png", "mimeType": "image/png", "prompt": "p" });
    save(&dir, &bad_image);
    let store = FileArticleStore::new(&dir.0);

    let good = serde_json::to_value(store.get("good").await.unwrap().unwrap()).unwrap();
    assert_eq!(good["markdown"], "");
    assert_eq!(good["wordCount"], 0);
    assert_eq!(good["images"], json!([]));
    assert_eq!(good["problems"], json!([]));
    assert!(store.get("bad-status").await.unwrap().is_none());
    assert!(store.get("bad-image").await.unwrap().is_none());
    assert!(store.get("missing").await.unwrap().is_none());
    assert_eq!(store.list(true, 50).await.unwrap().len(), 1);
}

#[tokio::test]
async fn never_reads_outside_its_directory() {
    let dir = Scratch::new("articles-traversal");
    dir.write("secret/article.json", serde_json::to_vec(&record("secret", "published", "x")).unwrap());
    dir.write("secret/cover.png", b"png");
    let store = FileArticleStore::new(dir.0.join("articles"));
    std::fs::create_dir_all(dir.0.join("articles/a")).unwrap();

    for slug in ["../secret", "..", "A", "a/b", ""] {
        assert!(store.get(slug).await.unwrap().is_none(), "{slug}");
        assert!(store.media(slug, "cover.png").await.unwrap().is_none(), "{slug}");
    }
    for file in ["../../secret/cover.png", "article.json", "cover.PNG", "cover.gif", ".png"] {
        assert!(store.media("a", file).await.unwrap().is_none(), "{file}");
    }
}

#[tokio::test]
async fn serves_a_picture_with_its_type() {
    let dir = Scratch::new("articles-media");
    dir.write("a/cover.png", [0x89, 0x50, 0x4e, 0x47]);
    dir.write("a/s-1.jpg", b"jpg");
    dir.write("a/s-2.webp", b"webp");
    let store = FileArticleStore::new(&dir.0);

    assert_eq!(
        store.media("a", "cover.png").await.unwrap(),
        Some(Media { data: vec![0x89, 0x50, 0x4e, 0x47], mime_type: "image/png" })
    );
    assert_eq!(store.media("a", "s-1.jpg").await.unwrap().unwrap().mime_type, "image/jpeg");
    assert_eq!(store.media("a", "s-2.webp").await.unwrap().unwrap().mime_type, "image/webp");
    assert!(store.media("a", "other.png").await.unwrap().is_none());
}

#[tokio::test]
async fn is_empty_when_the_directory_is_not_there() {
    let dir = Scratch::new("articles-none");
    let store = FileArticleStore::new(dir.0.join("nope"));
    assert!(store.list(true, 50).await.unwrap().is_empty());
}
