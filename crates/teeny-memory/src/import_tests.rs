//! The import, from a mocked legacy CortexDB into the reference engine.

use std::sync::Arc;

use serde_json::json;
use teeny_core::Secret;
use tinymemory_api::conformance::ReferenceEngine;
use tinymemory_api::{Hit, ItemKind, ListRequest, MemoryEngine, MetaFilter};
use wiremock::matchers::{header, method, path, query_param, query_param_is_missing};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::EngineStore;

const ROOT: &str = "team:teeny";
const SCOPE: &str = "agent:tiny-x-bot";

fn legacy(server: &MockServer) -> CortexMemory {
    CortexMemory::new(&server.uri(), "agent:teeny", Secret::new("tok"), SCOPE).unwrap()
}

fn target(engine: &Arc<ReferenceEngine>) -> TinyMemory {
    TinyMemory::new(Arc::new(EngineStore::new(engine.clone(), ROOT).unwrap()))
}

fn options() -> ImportOptions {
    ImportOptions {
        backoff: Duration::from_millis(1),
        ..ImportOptions::default()
    }
}

async fn everything(engine: &ReferenceEngine) -> Vec<Hit> {
    engine
        .list(ListRequest::new(MetaFilter::default(), 100))
        .await
        .unwrap()
        .items
}

fn mention(id: &str, text: &str) -> serde_json::Value {
    json!({
        "id": id,
        "scope": SCOPE,
        "content": { "kind": "message", "role": "user", "text": text },
        "context": {
            "labels": ["x", "mention", "@ana", "thread:55"],
            "observed_at": "2026-09-12T23:00:00.000Z",
        },
        "observed_actor": { "id": "user:x-7", "type": "person" },
        "idempotency_key": format!("x:mention:2:{id}"),
    })
}

fn reply(id: &str) -> serde_json::Value {
    json!({
        "id": id,
        "content": { "kind": "message", "role": "assistant", "text": "gm fren" },
        "context": {
            "labels": ["x", "reply", "posted", "@ana"],
            "observed_at": "2026-09-13T00:00:00Z",
        },
        "observed_actor": "user:x-7",
    })
}

fn chart(id: &str) -> serde_json::Value {
    json!({
        "id": id,
        "content": { "kind": "json", "data": { "kind": "chart_post", "payload": { "text": "candles" } } },
        "context": { "labels": ["x", "activity", "chart_post"], "observed_at": "2026-08-01T00:00:00Z" },
        "idempotency_key": format!("x:chart_post:2:{id}"),
    })
}

fn page(items: Vec<serde_json::Value>, next: Option<&str>) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "items": items,
        "has_more": next.is_some(),
        "next_cursor": next,
    }))
}

/// Two pages: two mentions (each listed twice, as the store does), then a
/// reply and a chart post.
async fn two_pages(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/v1/events"))
        .and(query_param("scope", SCOPE))
        .and(query_param("view", "local"))
        .and(query_param("labels", "x,telegram"))
        .and(header("x-cortex-actor", "agent:teeny"))
        .and(header("authorization", "Bearer tok"))
        .and(query_param_is_missing("cursor"))
        .respond_with(page(
            vec![
                mention("e4", "gm teeny"),
                mention("e4", "gm teeny"),
                mention("e3", "wen chart"),
            ],
            Some("c2"),
        ))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/events"))
        .and(query_param("cursor", "c2"))
        .respond_with(page(vec![reply("e2"), chart("e1")], None))
        .mount(server)
        .await;
}

#[tokio::test]
async fn copies_every_event_with_its_text_labels_time_person_and_key() {
    let server = MockServer::start().await;
    two_pages(&server).await;
    let engine = Arc::new(ReferenceEngine::new());
    let memory = target(&engine);
    let legacy = legacy(&server);
    let report = Importer::new(&legacy, &memory, ROOT, SCOPE, options())
        .run(None)
        .await
        .unwrap();

    assert!(report.finished);
    assert_eq!(report.pages, 2);
    assert_eq!(report.kinds["mention"].read, 2);
    assert_eq!(report.kinds["mention"].stored, 2);
    assert_eq!(report.kinds["reply"].stored, 1);
    assert_eq!(report.kinds["chart_post"].stored, 1);
    assert_eq!(report.total().stored, 4);

    let items = everything(&engine).await;
    assert_eq!(items.len(), 4);
    let gm = items
        .iter()
        .find(|item| item.meta.source.id.as_deref() == Some("x:mention:2:e4"))
        .unwrap();
    assert_eq!(gm.kind, ItemKind::Conversation);
    assert_eq!(gm.text, "user: gm teeny");
    assert_eq!(gm.meta.tags, ["x", "mention", "@ana", "thread:55"]);
    assert_eq!(gm.meta.namespace.to_string(), ROOT);
    assert_eq!(
        gm.meta.observed_at.map(|at| at.to_rfc3339()),
        Some("2026-09-12T23:00:00+00:00".to_owned())
    );
    assert_eq!(
        gm.meta
            .observed_actor
            .as_ref()
            .map(|actor| actor.id.as_str()),
        Some("user:x-7")
    );
    let reply = items
        .iter()
        .find(|item| item.meta.tags.contains(&"reply".to_owned()))
        .unwrap();
    assert_eq!(reply.text, "assistant: gm fren");
    // Listed without its key, it is keyed by its event.
    assert_eq!(reply.meta.source.id.as_deref(), Some("cortex:e2"));
    assert_eq!(
        reply
            .meta
            .observed_actor
            .as_ref()
            .map(|actor| actor.id.as_str()),
        Some("user:x-7")
    );
    let chart = items
        .iter()
        .find(|item| item.kind == ItemKind::Document)
        .unwrap();
    assert_eq!(chart.meta.source.id.as_deref(), Some("x:chart_post:2:e1"));
    assert!(chart.text.contains("candles"));
}

#[tokio::test]
async fn an_imported_capture_and_a_live_one_are_the_same_item() {
    let server = MockServer::start().await;
    two_pages(&server).await;
    let engine = Arc::new(ReferenceEngine::new());
    let memory = target(&engine);
    // The Rust bot captured the same mention, the same way, before the import.
    crate::Memory::capture_mention(
        &memory,
        &crate::MemoryPost {
            id: "e4".into(),
            text: "gm teeny".into(),
            author_id: Some("7".into()),
            author_username: Some("ana".into()),
            conversation_id: Some("55".into()),
            created_at: Some("2026-09-12T23:00:00.000Z".into()),
        },
    )
    .await
    .unwrap();
    let legacy = legacy(&server);
    let report = Importer::new(&legacy, &memory, ROOT, SCOPE, options())
        .run(None)
        .await
        .unwrap();
    assert_eq!(report.kinds["mention"].already, 1);
    assert_eq!(everything(&engine).await.len(), 4);
}

#[tokio::test]
async fn a_rerun_stores_nothing_twice() {
    let server = MockServer::start().await;
    two_pages(&server).await;
    let engine = Arc::new(ReferenceEngine::new());
    let memory = target(&engine);
    let legacy = legacy(&server);
    let importer = Importer::new(&legacy, &memory, ROOT, SCOPE, options());
    importer.run(None).await.unwrap();
    let again = importer.run(None).await.unwrap();
    assert_eq!(again.total().stored, 0);
    assert_eq!(again.total().already, 4);
    assert_eq!(everything(&engine).await.len(), 4);
}

#[tokio::test]
async fn a_finished_run_leaves_a_cursor_the_next_run_stops_at() {
    let dir = tempdir();
    let file = dir.join("cursor.json");
    let server = MockServer::start().await;
    two_pages(&server).await;
    let engine = Arc::new(ReferenceEngine::new());
    let memory = target(&engine);
    let legacy = legacy(&server);
    Importer::new(&legacy, &memory, ROOT, SCOPE, options())
        .run(Some(&file))
        .await
        .unwrap();
    let cursor = ImportCursor::load(&file).unwrap();
    assert_eq!(cursor.done_head.as_deref(), Some("e4"));
    assert_eq!(cursor.resume, None);
    assert_eq!((cursor.root.as_str(), cursor.scope.as_str()), (ROOT, SCOPE));

    // Something new arrived; the run reads it and stops at the old head,
    // never asking for the second page.
    let newer = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/events"))
        .and(query_param_is_missing("cursor"))
        .respond_with(page(
            vec![mention("e5", "still here"), mention("e4", "gm teeny")],
            Some("c2"),
        ))
        .expect(1)
        .mount(&newer)
        .await;
    Mock::given(method("GET"))
        .and(query_param("cursor", "c2"))
        .respond_with(page(vec![], None))
        .expect(0)
        .mount(&newer)
        .await;
    let legacy =
        super::super::CortexMemory::new(&newer.uri(), "agent:teeny", Secret::new("tok"), SCOPE)
            .unwrap();
    let report = Importer::new(&legacy, &memory, ROOT, SCOPE, options())
        .run(Some(&file))
        .await
        .unwrap();
    assert_eq!(report.total().read, 1);
    assert_eq!(report.total().stored, 1);
    assert_eq!(
        ImportCursor::load(&file).unwrap().done_head.as_deref(),
        Some("e5")
    );
}

#[tokio::test]
async fn a_run_that_fails_part_way_resumes_from_the_page_it_stopped_at() {
    let dir = tempdir();
    let file = dir.join("cursor.json");
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/events"))
        .and(query_param_is_missing("cursor"))
        .respond_with(page(
            vec![mention("e4", "gm"), mention("e3", "wen")],
            Some("c2"),
        ))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(query_param("cursor", "c2"))
        .respond_with(ResponseTemplate::new(503))
        .expect(4)
        .up_to_n_times(4)
        .mount(&server)
        .await;
    let engine = Arc::new(ReferenceEngine::new());
    let memory = target(&engine);
    let legacy = legacy(&server);
    let importer = Importer::new(&legacy, &memory, ROOT, SCOPE, options());
    let error = importer.run(Some(&file)).await.unwrap_err();
    assert!(
        matches!(error, MemoryError::Api { status: 503, .. }),
        "{error}"
    );
    let cursor = ImportCursor::load(&file).unwrap();
    assert_eq!(cursor.resume.as_deref(), Some("c2"));
    assert_eq!(cursor.run_head.as_deref(), Some("e4"));
    assert_eq!(cursor.done_head, None);

    // The store is back: the run goes on from the second page only.
    Mock::given(method("GET"))
        .and(query_param("cursor", "c2"))
        .respond_with(page(vec![reply("e2")], None))
        .mount(&server)
        .await;
    let report = importer.run(Some(&file)).await.unwrap();
    assert_eq!(report.pages, 1);
    assert_eq!(report.total().stored, 1);
    assert_eq!(everything(&engine).await.len(), 3);
    let cursor = ImportCursor::load(&file).unwrap();
    assert_eq!(cursor.done_head.as_deref(), Some("e4"));
    assert_eq!(cursor.resume, None);
}

#[tokio::test]
async fn a_slow_or_overloaded_listing_is_retried_and_a_refusal_is_not() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(429))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .respond_with(page(vec![mention("e1", "gm")], None))
        .mount(&server)
        .await;
    let engine = Arc::new(ReferenceEngine::new());
    let memory = target(&engine);
    let legacy = legacy(&server);
    let report = Importer::new(&legacy, &memory, ROOT, SCOPE, options())
        .run(None)
        .await
        .unwrap();
    assert_eq!(report.total().stored, 1);

    let refused = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "error_code": "UNAUTHORIZED", "message": "bad key"
        })))
        .expect(1)
        .mount(&refused)
        .await;
    let legacy = CortexMemory::new(&refused.uri(), "a:b", Secret::new("x"), SCOPE).unwrap();
    let error = Importer::new(&legacy, &memory, ROOT, SCOPE, options())
        .run(None)
        .await
        .unwrap_err();
    assert!(
        matches!(error, MemoryError::Api { status: 401, .. }),
        "{error}"
    );
}

#[tokio::test]
async fn a_listing_past_its_deadline_is_a_timeout() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(page(vec![], None).set_delay(Duration::from_millis(500)))
        .mount(&server)
        .await;
    let engine = Arc::new(ReferenceEngine::new());
    let memory = target(&engine);
    let legacy = legacy(&server);
    let error = Importer::new(
        &legacy,
        &memory,
        ROOT,
        SCOPE,
        ImportOptions {
            request_timeout: Duration::from_millis(50),
            attempts: 2,
            ..options()
        },
    )
    .run(None)
    .await
    .unwrap_err();
    assert!(error.is_timeout(), "{error}");
}

#[tokio::test]
async fn a_dry_run_counts_and_writes_nothing() {
    let dir = tempdir();
    let file = dir.join("cursor.json");
    let server = MockServer::start().await;
    two_pages(&server).await;
    let engine = Arc::new(ReferenceEngine::new());
    let memory = target(&engine);
    let legacy = legacy(&server);
    let report = Importer::new(
        &legacy,
        &memory,
        ROOT,
        SCOPE,
        ImportOptions {
            dry_run: true,
            ..options()
        },
    )
    .run(Some(&file))
    .await
    .unwrap();
    assert_eq!(report.total().stored, 4);
    assert!(report.render().contains("dry run"));
    assert!(everything(&engine).await.is_empty());
    assert!(!file.exists());
}

#[tokio::test]
async fn since_leaves_out_what_was_observed_before_it() {
    let server = MockServer::start().await;
    two_pages(&server).await;
    let engine = Arc::new(ReferenceEngine::new());
    let memory = target(&engine);
    let legacy = legacy(&server);
    let report = Importer::new(
        &legacy,
        &memory,
        ROOT,
        SCOPE,
        ImportOptions {
            since: Some("2026-09-01T00:00:00Z".parse().unwrap()),
            ..options()
        },
    )
    .run(None)
    .await
    .unwrap();
    assert_eq!(report.kinds["chart_post"].skipped, 1);
    assert_eq!(report.total().stored, 3);
}

#[tokio::test]
async fn a_cursor_for_another_root_is_refused() {
    let dir = tempdir();
    let file = dir.join("cursor.json");
    ImportCursor {
        root: "team:other".into(),
        scope: SCOPE.into(),
        ..ImportCursor::default()
    }
    .save(&file)
    .unwrap();
    let server = MockServer::start().await;
    let engine = Arc::new(ReferenceEngine::new());
    let memory = target(&engine);
    let legacy = legacy(&server);
    let error = Importer::new(&legacy, &memory, ROOT, SCOPE, options())
        .run(Some(&file))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("team:other"), "{error}");
}

#[test]
fn the_default_cursor_lives_under_state_named_for_its_root() {
    assert_eq!(
        cursor_path(Path::new("/data"), "team:teeny"),
        Path::new("/data/state/memory-import-team-teeny.json")
    );
}

#[test]
fn an_event_reads_as_the_experience_it_was_captured_from() {
    let event = LegacyEvent::parse(&mention("e9", "hello")).unwrap();
    assert_eq!(event.kind(), "mention");
    let experience = event.experience.unwrap();
    assert_eq!(experience.idempotency_key, "x:mention:2:e9");
    assert_eq!(experience.observed_at, "2026-09-12T23:00:00.000Z");
    assert_eq!(experience.observed_actor.as_deref(), Some("user:x-7"));
    assert!(LegacyEvent::parse(&json!({ "content": {} })).is_none());
    let unlabelled =
        LegacyEvent::parse(&json!({ "id": "e", "content": { "kind": "image" } })).unwrap();
    assert_eq!(unlabelled.kind(), "unlabelled");
    assert!(unlabelled.experience.is_none());
}

/// A fresh directory for one test's cursor file.
fn tempdir() -> PathBuf {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "teeny-memory-import-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
