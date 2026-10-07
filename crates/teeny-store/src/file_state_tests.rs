use std::path::PathBuf;

use super::*;

fn scratch(name: &str) -> PathBuf {
    std::env::temp_dir()
        .join(format!(
            "teeny-store-{}-{}",
            std::process::id(),
            crate::support::random_u64()
        ))
        .join(name)
}

#[tokio::test]
async fn round_trips_every_field_so_the_poll_window_survives_a_restart() {
    let path = scratch("state.json");
    let store = FileStateStore::new(&path);
    let state = BotState {
        since_id: Some("42".into()),
        last_polled_at: Some("2026-09-01T22:00:00.000Z".into()),
        replied_post_ids: vec!["1".into(), "2".into()],
    };

    store.save(&state).await.unwrap();
    // A field saved but not loaded reads as absent, and the job silently
    // falls back to its lookback default on every run instead of the last poll.
    assert_eq!(store.load().await.unwrap(), state);
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[tokio::test]
async fn returns_empty_state_when_the_file_does_not_exist() {
    let store = FileStateStore::new(scratch("definitely-missing/state.json"));
    assert_eq!(store.load().await.unwrap(), BotState::default());
}

#[tokio::test]
async fn writes_the_typescript_spelling_and_leaves_no_temporary_file() {
    let path = scratch("nested/state.json");
    let store = FileStateStore::new(&path);
    store
        .save(&BotState {
            since_id: Some("7".into()),
            last_polled_at: None,
            replied_post_ids: vec![],
        })
        .await
        .unwrap();

    let written = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        written,
        "{\n  \"sinceId\": \"7\",\n  \"repliedPostIds\": []\n}\n"
    );
    assert!(!path.with_extension("json.tmp").exists());
    let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
}

#[tokio::test]
async fn reads_a_partial_file_and_treats_a_blank_cursor_as_absent() {
    let path = scratch("state.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        r#"{"sinceId":"","lastPolledAt":"2026-09-01T00:00:00.000Z"}"#,
    )
    .unwrap();

    let state = FileStateStore::new(&path).load().await.unwrap();
    assert_eq!(
        state,
        BotState {
            since_id: None,
            last_polled_at: Some("2026-09-01T00:00:00.000Z".into()),
            replied_post_ids: vec![]
        }
    );
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[tokio::test]
async fn refuses_a_file_that_is_not_json() {
    let path = scratch("state.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "not json").unwrap();
    assert!(FileStateStore::new(&path).load().await.is_err());
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[tokio::test]
async fn the_in_memory_store_starts_empty_and_keeps_what_it_was_given() {
    let store = InMemoryStateStore::default();
    assert_eq!(store.load().await.unwrap(), BotState::default());
    let state = BotState {
        since_id: Some("9".into()),
        ..BotState::default()
    };
    store.save(&state).await.unwrap();
    assert_eq!(store.load().await.unwrap(), state);
}
