//! A port of `legacy/test/telegram-inbox.test.ts` and `splitForTelegram`.

use serde_json::json;
use teeny_memory::ChatKind;

use super::*;

fn bot() -> BotIdentity {
    BotIdentity {
        id: 99,
        username: "teenybot".into(),
    }
}

fn message(value: serde_json::Value) -> Message {
    let mut base = json!({
        "message_id": 10,
        "date": 1_790_000_000,
        "chat": { "id": -100, "type": "supergroup", "title": "holders" },
        "from": { "id": 7, "is_bot": false, "first_name": "Alice", "last_name": "B", "username": "alice" },
    });
    base.as_object_mut()
        .unwrap()
        .extend(value.as_object().unwrap().clone());
    serde_json::from_value(base).unwrap()
}

#[test]
fn everything_in_a_dm_is_for_the_account() {
    let dm = message(json!({ "chat": { "id": 7, "type": "private" }, "text": "hi" }));
    assert!(is_addressed(&dm, &bot()));
}

#[test]
fn in_a_group_only_a_mention_a_reply_or_a_command_to_it() {
    assert!(!is_addressed(
        &message(json!({ "text": "hello room" })),
        &bot()
    ));
    let named = message(json!({
        "text": "hey @TeenyBot what is $TINY?",
        "entities": [{ "type": "mention", "offset": 4, "length": 9 }]
    }));
    assert!(is_addressed(&named, &bot()));
    let other = message(json!({
        "text": "hey @someoneelse",
        "entities": [{ "type": "mention", "offset": 4, "length": 12 }]
    }));
    assert!(!is_addressed(&other, &bot()));
    let reply = message(json!({
        "text": "true",
        "reply_to_message": { "message_id": 9, "date": 1, "chat": { "id": -100, "type": "supergroup" },
            "from": { "id": 99, "is_bot": true, "first_name": "Teeny" } }
    }));
    assert!(is_addressed(&reply, &bot()));
    assert!(is_addressed(
        &message(json!({ "text": "/start@teenybot" })),
        &bot()
    ));
    // UTF-16 offsets: an emoji before the mention counts two.
    let emoji = message(json!({
        "text": "🔥 @teenybot",
        "entities": [{ "type": "mention", "offset": 3, "length": 9 }]
    }));
    assert!(is_addressed(&emoji, &bot()));
}

#[test]
fn turns_a_persons_line_into_a_chat_message_without_the_handle() {
    let named = message(json!({ "text": "@teenybot   what is $TINY?", "message_thread_id": 3 }));
    let chat = to_chat_message(&named, &bot()).unwrap();
    assert_eq!(chat.id, "10");
    assert_eq!(chat.chat_id, "-100");
    assert_eq!(chat.chat_kind, ChatKind::Group);
    assert_eq!(chat.chat_title.as_deref(), Some("holders"));
    assert_eq!(chat.text, "what is $TINY?");
    assert_eq!(chat.author_id, "7");
    assert_eq!(chat.author_username.as_deref(), Some("alice"));
    assert_eq!(chat.author_name.as_deref(), Some("Alice B"));
    assert_eq!(chat.created_at.as_deref(), Some("2026-09-21T14:13:20.000Z"));
    assert_eq!(chat.message_thread_id.as_deref(), Some("3"));
    assert_eq!(
        to_chat_message(&message(json!({ "text": "/start@teenybot go" })), &bot())
            .unwrap()
            .text,
        "/start go"
    );
}

#[test]
fn a_picture_is_written_in_as_unseen() {
    let photo = message(json!({ "caption": "look", "photo": [{ "file_id": "a" }] }));
    assert_eq!(
        to_chat_message(&photo, &bot()).unwrap().text,
        "look\n[sent a photo]"
    );
}

#[test]
fn ignores_bots_channels_service_messages_its_own_lines_and_empty_ones() {
    let bot_user = message(
        json!({ "text": "BUY", "from": { "id": 5, "is_bot": true, "first_name": "buybot" } }),
    );
    assert!(to_chat_message(&bot_user, &bot()).is_none());
    let channel =
        message(json!({ "text": "news", "sender_chat": { "id": -5, "type": "channel" } }));
    assert!(to_chat_message(&channel, &bot()).is_none());
    let forward = message(json!({ "text": "news", "is_automatic_forward": true }));
    assert!(to_chat_message(&forward, &bot()).is_none());
    let joined =
        message(json!({ "new_chat_members": [{ "id": 8, "is_bot": false, "first_name": "Bo" }] }));
    assert!(to_chat_message(&joined, &bot()).is_none());
    let pinned = message(json!({ "text": "x", "pinned_message": {} }));
    assert!(to_chat_message(&pinned, &bot()).is_none());
    let own =
        message(json!({ "text": "x", "from": { "id": 99, "is_bot": false, "first_name": "T" } }));
    assert!(to_chat_message(&own, &bot()).is_none());
    assert!(to_chat_message(&message(json!({ "text": "@teenybot" })), &bot()).is_none());
}

#[test]
fn reads_the_command_without_the_bot_suffix() {
    assert_eq!(
        command_of(&message(json!({ "text": "/Start@teenybot hi" }))).as_deref(),
        Some("start")
    );
    assert_eq!(
        command_of(&message(json!({ "text": "/help" }))).as_deref(),
        Some("help")
    );
    assert_eq!(command_of(&message(json!({ "text": "hi /help" }))), None);
}

#[test]
fn splits_a_long_reply_at_paragraphs_then_lines_then_words() {
    assert_eq!(split_for_telegram("  short  ", 4_096), ["short"]);
    assert!(split_for_telegram("   ", 4_096).is_empty());
    let text = format!("{}\n\n{}", "a".repeat(30), "b".repeat(30));
    assert_eq!(
        split_for_telegram(&text, 40),
        ["a".repeat(30), "b".repeat(30)]
    );
    // The window is the first `limit` characters, so a word that ends the
    // window exactly still breaks at the space before it, as the legacy did.
    let words = "one two three four five six";
    assert_eq!(
        split_for_telegram(words, 10),
        ["one two", "three", "four five", "six"]
    );
    // No break past the first quarter: a hard cut.
    assert_eq!(
        split_for_telegram(&"x".repeat(25), 10),
        ["x".repeat(10), "x".repeat(10), "x".repeat(5)]
    );
}

#[test]
fn the_pictures_to_look_at_are_the_largest_photo_size_and_an_image_document() {
    let both = message(json!({
        "caption": "look",
        "photo": [
            { "file_id": "small", "width": 90, "height": 90 },
            { "file_id": "large", "width": 800, "height": 600 },
            { "file_id": "medium", "width": 320, "height": 240 }
        ],
        "document": { "file_id": "doc", "mime_type": "image/png" }
    }));
    assert_eq!(picture_files(&both), ["large", "doc"]);
    let pdf = message(json!({
        "caption": "a paper",
        "document": { "file_id": "pdf", "mime_type": "application/pdf" }
    }));
    assert!(picture_files(&pdf).is_empty());
    assert_eq!(
        with_pictures_seen(
            &both,
            &bot(),
            vec!["[image: a]".into(), "[image: b]".into()]
        )
        .unwrap()
        .text,
        "look\n[image: a]\n[image: b]"
    );
}
