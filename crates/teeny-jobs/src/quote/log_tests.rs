use super::*;

fn entry(id: &str, at: &str) -> QuotedPost {
    QuotedPost {
        post_id: id.into(),
        author_username: Some("a".into()),
        quoted_at: at.into(),
        own_post_id: None,
    }
}

#[tokio::test]
async fn starts_empty_appends_replaces_a_repeat_and_survives_a_reread() {
    let dir = std::env::temp_dir().join(format!("teeny-quote-log-{}", std::process::id()));
    let path = dir.join("nested").join("quotes.json");
    let log = FileQuoteLog::new(&path);
    assert!(log.list().await.unwrap().is_empty());
    log.record(entry("1", "2026-09-21T23:00:00.000Z"))
        .await
        .unwrap();
    log.record(QuotedPost {
        author_username: None,
        own_post_id: Some("own".into()),
        ..entry("2", "2026-09-22T23:00:00.000Z")
    })
    .await
    .unwrap();
    log.record(entry("1", "2026-09-23T23:00:00.000Z"))
        .await
        .unwrap();
    let ids: Vec<_> = FileQuoteLog::new(&path)
        .list()
        .await
        .unwrap()
        .into_iter()
        .map(|entry| entry.post_id)
        .collect();
    assert_eq!(ids, ["2", "1"]);
    let raw: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(raw["quoted"][0]["ownPostId"], "own");
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn the_memory_log_keeps_the_same_contract() {
    let log = MemoryQuoteLog::new(vec![entry("1", "a")]);
    log.record(entry("1", "b")).await.unwrap();
    assert_eq!(log.list().await.unwrap(), [entry("1", "b")]);
}
