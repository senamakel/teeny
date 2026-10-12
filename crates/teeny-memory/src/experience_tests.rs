use chrono::TimeZone;

use super::*;
use crate::person::ChatKind;

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 13, 7, 0, 0).unwrap()
}

fn post() -> MemoryPost {
    MemoryPost {
        id: "10".into(),
        text: "@bot much higher?".into(),
        author_id: Some("7".into()),
        author_username: Some("fren".into()),
        conversation_id: Some("9".into()),
        created_at: Some("2026-09-12T23:00:00.000Z".into()),
    }
}

fn message() -> MemoryChatMessage {
    MemoryChatMessage {
        id: "55".into(),
        channel: "telegram".into(),
        chat_id: "-100".into(),
        chat_kind: ChatKind::Group,
        text: "what does $TINY do".into(),
        author_id: "42".into(),
        author_username: Some("alice".into()),
        created_at: None,
    }
}

#[test]
fn people_are_namespaced_inside_the_user_actor_type() {
    assert_eq!(
        Person::x("7", Some("fren")),
        Person {
            id: "user:x-7".into(),
            username: Some("fren".into())
        }
    );
    assert_eq!(
        Person::chat("telegram", "42", None),
        Person {
            id: "user:telegram-42".into(),
            username: None
        }
    );
    assert_eq!(post().person(), Some(Person::x("7", Some("fren"))));
    assert_eq!(
        MemoryPost {
            author_id: None,
            ..post()
        }
        .person(),
        None
    );
    assert_eq!(
        message().person(),
        Person::chat("telegram", "42", Some("alice"))
    );
}

#[test]
fn an_activity_is_json_under_a_versioned_key() {
    let person = Person::x("7", Some("fren"));
    let experience = Experience::activity(
        "chart_post",
        "123",
        json!({ "text": "level held" }),
        Some(&person),
        now(),
    );
    assert_eq!(
        experience.envelope("agent:teeny"),
        json!({
            "scope": "agent:teeny",
            "modality": "conversation",
            "content": { "kind": "json", "data": { "kind": "chart_post", "payload": { "text": "level held" } } },
            "context": { "observed_at": "2026-09-13T07:00:00.000Z", "labels": ["x", "activity", "chart_post", "@fren"] },
            "idempotency_key": "x:chart_post:2:123",
            "observed_actor": { "id": "user:x-7", "type": "person" },
        })
    );
    let anonymous = Experience::activity("agent_note", "n1", json!(null), None, now());
    assert_eq!(anonymous.labels, ["x", "activity", "agent_note"]);
    assert_eq!(anonymous.observed_actor, None);
    assert!(anonymous.envelope("s").get("observed_actor").is_none());
}

#[test]
fn a_mention_is_a_turn_from_its_author() {
    assert_eq!(
        Experience::mention(&post(), now()).envelope("agent:teeny"),
        json!({
            "scope": "agent:teeny",
            "modality": "conversation",
            "content": { "kind": "message", "role": "user", "text": "@bot much higher?" },
            "context": { "observed_at": "2026-09-12T23:00:00.000Z", "labels": ["x", "mention", "@fren", "thread:9"] },
            "idempotency_key": "x:mention:2:10",
            "observed_actor": { "id": "user:x-7", "type": "person" },
        })
    );
    let bare = Experience::mention(
        &MemoryPost {
            id: "11".into(),
            text: "hi".into(),
            ..MemoryPost::default()
        },
        now(),
    );
    assert_eq!(bare.labels, ["x", "mention"]);
    assert_eq!(bare.observed_at, "2026-09-13T07:00:00.000Z");
    assert_eq!(bare.observed_actor, None);
}

#[test]
fn a_reply_is_the_accounts_turn_about_the_same_person() {
    let posted = Experience::reply(&post(), "we are green", Some("99"), now());
    assert_eq!(
        posted.content,
        Content::Message {
            role: Role::Assistant,
            text: "we are green".into()
        }
    );
    assert_eq!(posted.labels, ["x", "reply", "posted", "@fren", "thread:9"]);
    assert_eq!(posted.observed_at, "2026-09-13T07:00:00.000Z");
    assert_eq!(posted.idempotency_key, "x:reply:2:10:99");
    assert_eq!(posted.observed_actor.as_deref(), Some("user:x-7"));

    let dry = Experience::reply(&post(), "we are green", None, now());
    assert_eq!(dry.labels[2], "dry-run");
    assert_eq!(dry.idempotency_key, "x:reply:2:10:dry-run");
}

#[test]
fn a_chat_message_is_labelled_with_its_channel_not_scoped_by_it() {
    assert_eq!(
        Experience::chat_message(&message(), now()).envelope("agent:teeny"),
        json!({
            "scope": "agent:teeny",
            "modality": "conversation",
            "content": { "kind": "message", "role": "user", "text": "what does $TINY do" },
            "context": {
                "observed_at": "2026-09-13T07:00:00.000Z",
                "labels": ["telegram", "message", "group", "@alice", "chat:telegram--100"],
            },
            "idempotency_key": "telegram:message:2:-100:55",
            "observed_actor": { "id": "user:telegram-42", "type": "person" },
        })
    );
    let dm = MemoryChatMessage {
        chat_kind: ChatKind::Private,
        author_username: None,
        created_at: Some("t".into()),
        ..message()
    };
    let stored = Experience::chat_message(&dm, now());
    assert_eq!(
        stored.labels,
        ["telegram", "message", "private", "chat:telegram--100"]
    );
    assert_eq!(stored.observed_at, "t");
}

#[test]
fn a_chat_reply_is_sent_or_dry_run() {
    let sent = Experience::chat_reply(&message(), "a memecoin with a mascot", Some("77"), now());
    assert_eq!(
        sent.content,
        Content::Message {
            role: Role::Assistant,
            text: "a memecoin with a mascot".into()
        }
    );
    assert_eq!(
        sent.labels,
        ["telegram", "reply", "sent", "@alice", "chat:telegram--100"]
    );
    assert_eq!(sent.idempotency_key, "telegram:reply:2:-100:55:77");
    assert_eq!(sent.observed_actor.as_deref(), Some("user:telegram-42"));
    let dry = Experience::chat_reply(&message(), "x", None, now());
    assert_eq!(dry.labels[2], "dry-run");
    assert_eq!(dry.idempotency_key, "telegram:reply:2:-100:55:dry-run");
}
