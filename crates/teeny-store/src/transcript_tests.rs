use super::*;

fn line(n: usize, from: Speaker) -> ChatTurn {
    ChatTurn {
        from,
        author_name: (from == Speaker::Person).then(|| "Alice".to_owned()),
        text: format!("line {n} of the conversation"),
    }
}

fn person(n: usize) -> ChatTurn {
    line(n, Speaker::Person)
}

#[test]
fn keeps_the_newest_lines_that_fit_and_returns_them_oldest_first() {
    let newest_first = vec![person(5), person(4), person(3), person(2), person(1)];
    let per_line = estimate_tokens(&person(1).text) + 6;
    let kept = fit_to_budget(newest_first, per_line * 2 + 1);
    assert_eq!(kept, [person(4), person(5)]);
}

#[test]
fn keeps_a_single_oversized_newest_line_rather_than_nothing() {
    let huge = ChatTurn {
        from: Speaker::Person,
        author_name: None,
        text: "x".repeat(10_000),
    };
    assert_eq!(fit_to_budget(vec![huge.clone(), person(1)], 50), [huge]);
}

#[test]
fn keeps_everything_when_the_budget_allows() {
    assert_eq!(fit_to_budget(vec![person(2), person(1)], 5_000).len(), 2);
}

#[test]
fn counts_tokens_as_the_typescript_service_did() {
    assert_eq!(estimate_tokens(""), 0);
    assert_eq!(estimate_tokens("abcde"), 2);
    // UTF-16 code units, as `text.length` counts them: an emoji is two.
    assert_eq!(estimate_tokens("😀😀"), 1);
}

fn entry(
    chat: &str,
    message: &str,
    at: &str,
    from: Speaker,
    author: Option<&str>,
    text: &str,
) -> TranscriptEntry {
    TranscriptEntry {
        chat_id: chat.into(),
        message_id: message.into(),
        at: at.into(),
        from,
        author_name: author.map(Into::into),
        text: text.into(),
    }
}

#[tokio::test]
async fn stores_per_chat_drops_redeliveries_and_excludes_the_message_being_answered() {
    let store = InMemoryChatTranscript::default();
    let at = "2026-09-14T00:00:00.000Z";
    store
        .append(&entry("1", "a", at, Speaker::Person, Some("A"), "hi"))
        .await
        .unwrap();
    store
        .append(&entry("1", "a", at, Speaker::Person, Some("A"), "hi"))
        .await
        .unwrap();
    store
        .append(&entry("1", "b", at, Speaker::Bot, None, "hello"))
        .await
        .unwrap();
    store
        .append(&entry(
            "1",
            "c",
            at,
            Speaker::Person,
            Some("A"),
            "how are you",
        ))
        .await
        .unwrap();
    store
        .append(&entry(
            "2",
            "z",
            at,
            Speaker::Person,
            Some("B"),
            "other room",
        ))
        .await
        .unwrap();

    assert_eq!(
        store.recent("1", 5_000, &["c".into()]).await.unwrap(),
        [
            ChatTurn {
                from: Speaker::Person,
                author_name: Some("A".into()),
                text: "hi".into()
            },
            ChatTurn {
                from: Speaker::Bot,
                author_name: None,
                text: "hello".into()
            },
        ]
    );
    assert_eq!(
        store.recent("2", 5_000, &[]).await.unwrap(),
        [ChatTurn {
            from: Speaker::Person,
            author_name: Some("B".into()),
            text: "other room".into()
        }]
    );
}

#[test]
fn an_entry_is_spelled_as_the_typescript_service_stored_it() {
    assert_eq!(
        serde_json::to_value(entry("c", "1", "t", Speaker::Bot, None, "gm")).unwrap(),
        serde_json::json!({ "chatId": "c", "messageId": "1", "at": "t", "from": "bot", "text": "gm" })
    );
}

#[tokio::test]
#[ignore = "needs a Mongo at TEENY_TEST_MONGO_URI"]
async fn round_trips_turns_in_mongo_oldest_first_within_the_budget_and_survives_redelivery() {
    let Some(db) = crate::mongo::testing::database().await else {
        return;
    };
    let name = crate::mongo::testing::collection("transcript");
    let store = MongoChatTranscript::with_collection(&db, &name);
    for i in 1..=5 {
        let (from, author) = if i % 2 == 1 {
            (Speaker::Person, Some("Alice"))
        } else {
            (Speaker::Bot, None)
        };
        let at = format!("2026-09-14T00:00:0{i}.000Z");
        store
            .append(&entry(
                "c",
                &i.to_string(),
                &at,
                from,
                author,
                &format!("line {i}"),
            ))
            .await
            .unwrap();
    }
    store
        .append(&entry("c", "3", "later", Speaker::Person, None, "dup"))
        .await
        .unwrap();

    let texts = |turns: Vec<ChatTurn>| turns.into_iter().map(|turn| turn.text).collect::<Vec<_>>();
    assert_eq!(
        texts(store.recent("c", 5_000, &["5".into()]).await.unwrap()),
        ["line 1", "line 2", "line 3", "line 4"]
    );
    assert_eq!(
        texts(store.recent("c", 20, &[]).await.unwrap()),
        ["line 4", "line 5"]
    );
    db.collection::<mongodb::bson::Document>(&name)
        .drop()
        .await
        .unwrap();
}
