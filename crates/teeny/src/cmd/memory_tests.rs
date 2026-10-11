use std::sync::Arc;

use serde_json::json;
use teeny_core::Secret;
use teeny_memory::EngineStore;
use teeny_memory::tiny::api::conformance::ReferenceEngine;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

fn args(dry_run: bool) -> ImportArgs {
    ImportArgs {
        dry_run,
        since: None,
        cursor: None,
    }
}

async fn legacy_with_one_mention() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/events"))
        .and(query_param("labels", "x,telegram"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "items": [{
                "id": "e1",
                "content": { "kind": "message", "role": "user", "text": "gm" },
                "context": { "labels": ["x", "mention", "@ana"], "observed_at": "2026-09-01T00:00:00Z" },
                "idempotency_key": "x:mention:2:1",
            }],
            "has_more": false,
        })))
        .mount(&server)
        .await;
    server
}

fn cursor_file(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("teeny-cmd-memory-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir.join("cursor.json")
}

#[tokio::test]
async fn imports_into_the_shared_root_and_prints_counts_per_kind() {
    let server = legacy_with_one_mention().await;
    let legacy = CortexMemory::new(&server.uri(), "agent:t", Secret::new("k"), "agent:t").unwrap();
    let engine = Arc::new(ReferenceEngine::new());
    let memory = TinyMemory::new(Arc::new(EngineStore::new(engine, "team:teeny").unwrap()));
    let file = cursor_file("import");
    let mut out = Vec::new();
    let place = Place {
        scope: "agent:t".into(),
        legacy_url: server.uri(),
        root: "team:teeny".into(),
        url: "http://teeny-cortexdb:3141".into(),
    };
    run(&legacy, &memory, &place, &args(false), &file, &mut out)
        .await
        .unwrap();
    let printed = String::from_utf8(out).unwrap();
    assert!(
        printed.contains("agent:t at ")
            && printed.contains("team:teeny at http://teeny-cortexdb:3141"),
        "{printed}"
    );
    let mention = printed
        .lines()
        .find(|line| line.starts_with("mention"))
        .unwrap_or_else(|| panic!("no mention line: {printed}"));
    assert_eq!(
        mention.split_whitespace().collect::<Vec<_>>(),
        ["mention", "1", "1", "0", "0"]
    );
    assert!(printed.contains("finished"), "{printed}");
    assert!(file.exists(), "the cursor is kept");
}

#[tokio::test]
async fn a_dry_run_says_so_and_keeps_no_cursor() {
    let server = legacy_with_one_mention().await;
    let legacy = CortexMemory::new(&server.uri(), "agent:t", Secret::new("k"), "agent:t").unwrap();
    let engine = Arc::new(ReferenceEngine::new());
    let memory = TinyMemory::new(Arc::new(EngineStore::new(engine, "team:teeny").unwrap()));
    let file = cursor_file("dry");
    let mut out = Vec::new();
    let place = Place {
        scope: "agent:t".into(),
        legacy_url: server.uri(),
        root: "team:teeny".into(),
        url: "http://x".into(),
    };
    run(&legacy, &memory, &place, &args(true), &file, &mut out)
        .await
        .unwrap();
    let printed = String::from_utf8(out).unwrap();
    assert!(printed.contains("dry run"), "{printed}");
    assert!(!file.exists());
}

#[test]
fn since_takes_a_date_or_a_timestamp() {
    assert_eq!(
        parse_since("2026-09-01").unwrap().to_rfc3339(),
        "2026-09-01T00:00:00+00:00"
    );
    assert_eq!(
        parse_since("2026-09-01T12:30:00+02:00")
            .unwrap()
            .to_rfc3339(),
        "2026-09-01T10:30:00+00:00"
    );
    assert!(parse_since("yesterday").is_err());
}
