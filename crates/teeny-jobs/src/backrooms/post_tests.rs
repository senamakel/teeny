use teeny_agent::deps::Deps;
use teeny_x::XApi;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

pub(crate) async fn live_deps() -> (MockServer, Deps) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/2/tweets"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({ "data": { "id": "own-1", "text": "x" } })))
        .mount(&server)
        .await;
    let mut deps = Deps::fake();
    let config = teeny_core::Config::from_vars(teeny_agent::deps::fake_vars()).unwrap();
    deps.x = Arc::new(XApi::from_config(&config.x).with_base_url(server.uri()));
    deps.settings.dry_run = false;
    (server, deps)
}

#[tokio::test]
async fn posts_through_the_cadence_and_records_the_post() {
    let (server, deps) = live_deps().await;
    let published = publish_own(&deps, "lore", Direction::Outbound, "the kettle is on the ledger now.", None)
        .await
        .unwrap();
    assert_eq!(published.id.as_str(), "own-1");
    assert!(!published.dry_run);
    // The gap now holds the next own post.
    let again = publish_own(&deps, "lore", Direction::Outbound, "another page.", None).await;
    assert!(again.unwrap_err().to_string().contains("cadence: too soon"));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_dry_run_reaches_nothing_and_records_nothing() {
    let deps = Deps::fake();
    let published = publish_own(&deps, "lore", Direction::Outbound, "a page.", None).await.unwrap();
    assert!(published.dry_run);
    assert!(published.id.as_str().starts_with("dry-run-"));
    assert!(deps.cadence.claim("lore").await.unwrap().is_granted());
}

#[tokio::test]
async fn the_guard_refuses_what_the_lexicon_holds() {
    let (server, deps) = live_deps().await;
    let refused = publish_own(&deps, "lore", Direction::Outbound, "here is how to build a pipe bomb", None).await;
    assert!(refused.unwrap_err().to_string().contains("content filter"));
    assert!(server.received_requests().await.unwrap().is_empty());
}
