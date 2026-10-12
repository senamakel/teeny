use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::test_support::api;

/// Rewrites every draft, as house style would.
struct Shout;
#[async_trait]
impl PublishGuard for Shout {
    async fn check(&self, draft: &Draft) -> Result<Draft, Refusal> {
        Ok(Draft {
            text: draft.text.to_uppercase(),
            ..draft.clone()
        })
    }
}

struct Refuse;
#[async_trait]
impl PublishGuard for Refuse {
    async fn check(&self, _draft: &Draft) -> Result<Draft, Refusal> {
        Err(Refusal::new("names a ticker that is not ours"))
    }
}

async fn no_posts(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/2/tweets"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(server)
        .await;
}

fn publisher(server: &MockServer, guard: impl PublishGuard + 'static, dry_run: bool) -> XPublisher {
    XPublisher::new(Arc::new(api(server)), Arc::new(guard), dry_run)
}

#[tokio::test]
async fn posts_what_the_guard_returns() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/2/tweets"))
        .and(body_json(json!({ "text": "GM", "reply": { "in_reply_to_tweet_id": "7" }, "media": { "media_ids": ["m1"] } })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "data": { "id": "100" } })))
        .expect(1)
        .mount(&server)
        .await;
    let draft = Draft {
        reply_to: Some("7".into()),
        media_ids: vec!["m1".into()],
        ..Draft::new("gm")
    };
    let published = publisher(&server, Shout, false).post(draft).await.unwrap();
    assert_eq!(
        published,
        Published {
            id: "100".into(),
            dry_run: false,
            text: "GM".into(),
        }
    );
}

#[tokio::test]
async fn a_refusal_never_reaches_x() {
    let server = MockServer::start().await;
    no_posts(&server).await;
    let error = publisher(&server, Refuse, false)
        .post(Draft::new("buy $FAKE"))
        .await
        .unwrap_err();
    assert!(
        matches!(error, PublishError::Refused(ref refusal) if refusal.reason == "names a ticker that is not ours")
    );
}

#[tokio::test]
async fn refuses_over_280_as_x_counts_after_the_guard() {
    let server = MockServer::start().await;
    no_posts(&server).await;
    let x = publisher(&server, NoopGuard, true);
    assert!(x.post(Draft::new("a".repeat(280))).await.is_ok());
    assert!(matches!(
        x.post(Draft::new("a".repeat(281))).await,
        Err(PublishError::TooLong { length: 281 })
    ));
    // An emoji is two UTF-16 units, as the legacy `.length` counted it.
    assert!(x.post(Draft::new("🐸".repeat(140))).await.is_ok());
    assert!(matches!(
        x.post(Draft::new("🐸".repeat(141))).await,
        Err(PublishError::TooLong { length: 282 })
    ));
    // The guard's rewrite is what is measured.
    struct Pad;
    #[async_trait]
    impl PublishGuard for Pad {
        async fn check(&self, draft: &Draft) -> Result<Draft, Refusal> {
            Ok(Draft {
                text: format!("{}!", draft.text),
                ..draft.clone()
            })
        }
    }
    let padded = publisher(&server, Pad, true);
    assert!(matches!(
        padded.post(Draft::new("a".repeat(280))).await,
        Err(PublishError::TooLong { .. })
    ));
}

#[tokio::test]
async fn dry_run_runs_the_guard_and_publishes_nothing() {
    let server = MockServer::start().await;
    no_posts(&server).await;
    Mock::given(method("DELETE"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/2/articles/draft"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    let x = publisher(&server, Shout, true);
    let published = x.post(Draft::new("gm")).await.unwrap();
    assert!(published.dry_run);
    assert!(published.id.as_str().starts_with("dry-run-"));
    assert!(matches!(
        publisher(&server, Refuse, true).post(Draft::new("x")).await,
        Err(PublishError::Refused(_))
    ));
    assert!(!x.unsay(&"55".into()).await.unwrap());
    x.like(&"77".into()).await.unwrap();
    let article = crate::article::ArticleDraft {
        title: "t".into(),
        content_state: crate::article::ContentState {
            blocks: vec![],
            entities: vec![],
        },
        cover_media_id: None,
    };
    assert!(
        x.publish_article(&article)
            .await
            .unwrap()
            .post_id
            .as_str()
            .starts_with("dry-run-")
    );
}

#[tokio::test]
async fn unsays_a_post_when_live() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/2/tweets/55"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "data": { "deleted": true } })),
        )
        .expect(1)
        .mount(&server)
        .await;
    assert!(
        publisher(&server, NoopGuard, false)
            .unsay(&"55".into())
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn a_dry_run_says_what_would_have_gone_out() {
    let server = MockServer::start().await;
    no_posts(&server).await;
    let published = publisher(&server, Shout, true)
        .post(Draft::new("gm"))
        .await
        .unwrap();
    assert!(published.dry_run);
    assert_eq!(published.text, "GM");
}

/// Counts the times it is asked to attach, and hands back one media id.
fn attacher(
    calls: &Arc<std::sync::atomic::AtomicUsize>,
) -> impl FnOnce() -> std::future::Ready<Result<Vec<String>, PublishError>> + Send + use<> {
    let calls = Arc::clone(calls);
    move || {
        calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        std::future::ready(Ok(vec!["m9".to_owned()]))
    }
}

#[tokio::test]
async fn attaches_only_after_the_guard_and_only_for_real() {
    let server = MockServer::start().await;
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    no_posts(&server).await;

    let refused = publisher(&server, Refuse, false)
        .post_with_media(Draft::new("buy $FAKE"), attacher(&calls))
        .await;
    assert!(matches!(refused, Err(PublishError::Refused(_))));
    let rehearsed = publisher(&server, NoopGuard, true)
        .post_with_media(Draft::new("gm"), attacher(&calls))
        .await
        .unwrap();
    assert!(rehearsed.dry_run);
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[tokio::test]
async fn puts_the_attached_media_on_the_post() {
    let server = MockServer::start().await;
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    Mock::given(method("POST"))
        .and(path("/2/tweets"))
        .and(body_json(
            json!({ "text": "gm", "media": { "media_ids": ["m9"] } }),
        ))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "data": { "id": "101" } })))
        .expect(1)
        .mount(&server)
        .await;
    let published = publisher(&server, NoopGuard, false)
        .post_with_media(Draft::new("gm"), attacher(&calls))
        .await
        .unwrap();
    assert_eq!(published.id, "101".into());
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
}
