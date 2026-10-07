//! Ported from legacy/test/article-endpoint.test.ts.

use std::sync::Arc;

use axum::http::{StatusCode, header};
use serde_json::json;

use crate::http::article_store::FileArticleStore;
use crate::http::test_support::*;
use crate::http::{Services, router};

fn app(dir: &Scratch) -> axum::Router {
    router(Services { articles: Some(Arc::new(FileArticleStore::new(&dir.0))), ..services() })
}

fn the_inch(dir: &Scratch) {
    dir.write(
        "20260923-the-inch/article.json",
        json!({
            "slug": "20260923-the-inch", "title": "the inch", "dek": "a door", "status": "published",
            "createdAt": "2026-09-23T23:00:00.000Z", "publishedAt": "2026-09-23T23:05:00.000Z",
            "postId": "9001", "markdown": "## the gap\n\na", "wordCount": 3,
            "cover": { "key": "cover", "file": "cover.png", "mimeType": "image/png", "prompt": "p", "caption": "" },
            "problems": ["kept a picture the check did not like"], "critique": "Verdict: FLAT",
            "error": "e", "proof": { "ok": true }, "judge": { "score": 1 }, "contentState": { "blocks": [] },
        })
        .to_string(),
    );
    dir.write("20260923-the-inch/cover.png", [0x89, 0x50, 0x4e, 0x47]);
    dir.write(
        "20260920-rehearsal/article.json",
        json!({ "slug": "20260920-rehearsal", "title": "r", "dek": "d", "status": "dry-run", "createdAt": "2026-09-20T23:00:00.000Z" })
            .to_string(),
    );
}

#[tokio::test]
async fn serves_the_published_articles_and_every_rehearsal_on_request() {
    let dir = Scratch::new("route-list");
    the_inch(&dir);
    let app = app(&dir);

    let list = call(&app, get("/articles")).await;
    assert_eq!(list.status, StatusCode::OK);
    let body = list.json();
    assert_eq!(body["source"], "live");
    assert!(body["generatedAt"].is_string());
    assert_eq!(body["articles"].as_array().unwrap().len(), 1);
    assert_eq!(body["articles"][0]["slug"], "20260923-the-inch");
    assert_eq!(body["articles"][0]["cover"], "cover.png");
    assert_eq!(body["articles"][0]["postId"], "9001");

    assert_eq!(call(&app, get("/articles?all=1")).await.json()["articles"].as_array().unwrap().len(), 2);
    assert_eq!(call(&app, get("/articles?all=true")).await.json()["articles"].as_array().unwrap().len(), 2);
    assert_eq!(call(&app, get("/articles?all=yes")).await.json()["articles"].as_array().unwrap().len(), 1);
    assert_eq!(call(&app, get("/articles?all=1&limit=1")).await.json()["articles"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn serves_one_piece_without_the_operator_detail_unless_asked() {
    let dir = Scratch::new("route-one");
    the_inch(&dir);
    let app = app(&dir);

    let one = call(&app, get("/articles/20260923-the-inch")).await;
    assert_eq!(one.status, StatusCode::OK);
    let body = one.json();
    assert_eq!(body["title"], "the inch");
    assert_eq!(body["markdown"], "## the gap\n\na");
    assert_eq!(body["cover"]["file"], "cover.png");
    for private in ["problems", "error", "critique", "proof", "judge", "contentState"] {
        assert!(body.get(private).is_none(), "{private} leaked");
    }

    let whole = call(&app, get("/articles/20260923-the-inch?all=1")).await.json();
    assert_eq!(whole["problems"].as_array().unwrap().len(), 1);
    assert_eq!(whole["critique"], "Verdict: FLAT");

    let missing = call(&app, get("/articles/nope")).await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert_eq!(missing.json(), json!({ "error": "no such article" }));
}

#[tokio::test]
async fn serves_its_pictures_and_nothing_else() {
    let dir = Scratch::new("route-media");
    the_inch(&dir);
    let app = app(&dir);

    let cover = call(&app, get("/articles/20260923-the-inch/media/cover.png")).await;
    assert_eq!(cover.status, StatusCode::OK);
    assert_eq!(cover.headers[header::CONTENT_TYPE], "image/png");
    assert_eq!(cover.headers[header::CACHE_CONTROL], "public, max-age=86400");
    assert_eq!(cover.body.as_ref(), [0x89, 0x50, 0x4e, 0x47]);

    let other = call(&app, get("/articles/20260923-the-inch/media/other.png")).await;
    assert_eq!(other.status, StatusCode::NOT_FOUND);
    assert_eq!(other.json(), json!({ "error": "no such file" }));

    for traversal in [
        "/articles/20260923-the-inch/media/..%2F20260920-rehearsal%2Farticle.json",
        "/articles/20260923-the-inch/media/article.json",
        "/articles/..%2F..%2Fetc/media/passwd.png",
        "/articles/%2E%2E/media/cover.png",
    ] {
        assert_eq!(call(&app, get(traversal)).await.status, StatusCode::NOT_FOUND, "{traversal}");
    }
}

#[tokio::test]
async fn says_so_when_no_store_is_configured() {
    let app = router(services());
    for uri in ["/articles", "/articles/a", "/articles/a/media/b.png"] {
        let answer = call(&app, get(uri)).await;
        assert_eq!(answer.status, StatusCode::SERVICE_UNAVAILABLE, "{uri}");
        assert_eq!(answer.json(), json!({ "error": "no article store is configured" }));
    }
}
