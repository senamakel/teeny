//! Port of the delivery-parsing half of legacy/test/webhooks.test.ts.

use serde_json::{Value, json};

use super::*;

const BOT: &str = "2094854245468000256";

fn bot() -> UserId {
    BOT.into()
}

fn mentions(payload: &Value) -> Vec<XPost> {
    mentions_from_activity(payload, &bot(), "TinyMascot")
}

fn delivery(events: Value) -> Value {
    json!({ "for_user_id": BOT, "tweet_create_events": events })
}

fn event(event_type: &str, payload: Value, includes: Option<Value>) -> Value {
    let mut data = json!({
        "event_type": event_type,
        "event_uuid": "777",
        "filter": { "user_id": BOT },
        "tag": format!("teeny {event_type}"),
        "payload": payload,
    });
    if let Some(includes) = includes {
        data["includes"] = includes;
    }
    json!({ "data": data })
}

#[test]
fn queues_a_mention_using_the_full_text_of_a_truncated_post() {
    let payload = delivery(json!([{
        "id_str": "900",
        "text": "hey @TinyMascot this is going to be cut off at one hundred and forty…",
        "truncated": true,
        "extended_tweet": { "full_text": "hey @TinyMascot the whole thing" },
        "created_at": "Wed Sep 02 13:30:16 +0000 2026",
        "user": { "id_str": "42", "screen_name": "someone" }
    }]));
    assert_eq!(
        serde_json::to_value(mentions(&payload)).unwrap(),
        json!([{
            "id": "900", "text": "hey @TinyMascot the whole thing", "authorId": "42",
            "authorUsername": "someone", "createdAt": "2026-09-02T13:30:16.000Z"
        }])
    );
}

#[test]
fn never_takes_the_accounts_own_posts_or_retweets() {
    let own = delivery(
        json!([{ "id_str": "901", "text": "gm from the bot", "user": { "id_str": BOT, "screen_name": "TinyMascot" } }]),
    );
    assert!(mentions(&own).is_empty());
    let retweet = delivery(json!([{
        "id_str": "2095151478851633461",
        "text": "RT @TinyMascot: @senamakel alive and tiny as ever, sena",
        "retweeted_status": { "id_str": "2095142265874231486" },
        "user": { "id_str": "999", "screen_name": "Sorin_016" }
    }]));
    assert!(mentions(&retweet).is_empty());
}

#[test]
fn takes_nothing_when_the_account_is_unknown() {
    let payload = delivery(
        json!([{ "id_str": "902", "text": "hi @TinyMascot", "user": { "id_str": "42" } }]),
    );
    assert!(mentions_from_activity(&payload, &"".into(), "TinyMascot").is_empty());
}

#[test]
fn drops_a_deep_delivery_with_only_the_prefixed_handle_and_keeps_a_typed_one() {
    let prefixed = delivery(json!([{
        "id_str": "902", "text": "@iamgaurangdesai @TinyMascot wen ser",
        "in_reply_to_user_id_str": "999", "user": { "id_str": "42", "screen_name": "someone" }
    }]));
    assert!(mentions(&prefixed).is_empty());
    let typed = delivery(json!([{
        "id_str": "903", "text": "@senamakel Oh hi fren @TinyMascot",
        "in_reply_to_user_id_str": "999", "user": { "id_str": "42", "screen_name": "coldcoleonx" }
    }]));
    let found = mentions(&typed);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].author_username.as_deref(), Some("coldcoleonx"));
}

#[test]
fn reads_a_mention_out_of_the_activity_envelope_with_the_author_from_includes() {
    let payload = event(
        "post.mention.create",
        json!({ "id": "900", "text": "hey @TinyMascot", "author_id": "42", "created_at": "2026-09-14T09:40:00.000Z" }),
        Some(json!({ "users": [{ "id": "42", "username": "someone" }] })),
    );
    assert_eq!(
        serde_json::to_value(mentions(&payload)).unwrap(),
        json!([{ "id": "900", "text": "hey @TinyMascot", "authorId": "42", "authorUsername": "someone", "createdAt": "2026-09-14T09:40:00.000Z" }])
    );
}

#[test]
fn takes_a_reply_to_the_account_and_keeps_the_reply_target() {
    let payload = event(
        "post.reply.create",
        json!({ "id": "901", "text": "nice one", "author_id": "42", "in_reply_to_user_id": BOT, "conversation_id": "800" }),
        None,
    );
    assert_eq!(
        serde_json::to_value(mentions(&payload)).unwrap(),
        json!([{ "id": "901", "text": "nice one", "authorId": "42", "conversationId": "800", "inReplyToUserId": BOT }])
    );
}

#[test]
fn answers_neither_the_account_nor_an_event_that_is_not_a_post() {
    assert!(
        mentions(&event(
            "post.mention.create",
            json!({ "id": "902", "text": "@TinyMascot me", "author_id": BOT }),
            None
        ))
        .is_empty()
    );
    assert!(
        mentions(&event(
            "like.create",
            json!({ "id": "903", "text": "@TinyMascot", "author_id": "42" }),
            None
        ))
        .is_empty()
    );
    assert!(mentions(&event("post.mention.create", json!({}), None)).is_empty());
    assert!(
        mentions(&event(
            "post.quote.create",
            json!({ "id": "904", "text": "RT @someone: @TinyMascot", "author_id": "42" }),
            None
        ))
        .is_empty()
    );
}

#[test]
fn lists_entries_under_their_event_type_and_skips_posts_for_memory() {
    let follow = event(
        "follow.follow",
        json!({ "source": { "id": "42" }, "target": { "id": BOT } }),
        None,
    );
    assert_eq!(
        activity_entries(&follow),
        vec![ActivityEntry {
            kind: "follow.follow".into(),
            id: "777".into(),
            payload: json!({ "source": { "id": "42" }, "target": { "id": BOT } }),
        }]
    );
    let mention = event(
        "post.mention.create",
        json!({ "id": "904", "text": "@TinyMascot", "author_id": "42" }),
        None,
    );
    assert!(capturable_entries(&mention).is_empty());
    // Engagement is counted in the people store, not remembered: two thirds
    // of memory was raw like, repost and follow payloads crowding out what
    // people said.
    for (kind, payload) in [
        ("like.create", json!({ "id": "905", "author_id": "42" })),
        (
            "follow.follow",
            json!({ "source": { "id": "42" }, "target": { "id": "1" } }),
        ),
        (
            "post.repost.create",
            json!({ "id": "907", "author_id": "42" }),
        ),
    ] {
        assert!(
            capturable_entries(&event(kind, payload, None)).is_empty(),
            "{kind}"
        );
    }
    let dm = event("dm.received", json!({ "id": "908", "text": "hi" }), None);
    assert_eq!(
        capturable_entries(&dm),
        vec![ActivityEntry {
            kind: "dm.received".into(),
            id: "908".into(),
            payload: json!({ "id": "908", "text": "hi" })
        }]
    );
}

#[test]
fn neither_posts_nor_engagement_of_the_old_shape_reach_memory() {
    let payload = json!({
        "tweet_create_events": [{ "id_str": "123", "text": "hello" }],
        "favorite_events": [{ "id": "456" }],
        "follow_events": [{ "type": "follow" }],
        "direct_message_events": [{ "id": "789" }]
    });
    assert_eq!(
        capturable_entries(&payload),
        vec![ActivityEntry {
            kind: "direct_message_events".into(),
            id: "789".into(),
            payload: json!({ "id": "789" })
        }]
    );
}

#[test]
fn engagement_is_likes_reposts_follows_blocks_mutes_and_deletions() {
    for kind in [
        "like.create",
        "favorite.create",
        "post.repost.create",
        "repost.create",
        "retweet.create",
        "follow.follow",
        "unfollow.unfollow",
        "block.block",
        "mute.mute",
        "tweet_delete.create",
        "favorite_events",
        "follow_events",
        "block_events",
        "mute_events",
        "tweet_delete_events",
        "retweet_events",
    ] {
        assert!(is_engagement(kind), "{kind}");
    }
    for kind in [
        "dm.received",
        "post.mention.create",
        "direct_message_events",
        "likes",
        "followers",
    ] {
        assert!(!is_engagement(kind), "{kind}");
    }
}

#[test]
fn credits_the_author_the_follower_or_the_liker_never_the_account() {
    let quote = event(
        "post.quote.create",
        json!({ "id": "1", "author_id": "42" }),
        Some(json!({ "users": [{ "id": "42", "username": "q" }] })),
    );
    assert_eq!(
        interactions_from_activity(&quote, &bot()),
        vec![ActivityInteraction {
            user_id: "42".into(),
            username: Some("q".into()),
            kind: InteractionKind::Quote
        }]
    );
    let repost = event(
        "post.repost.create",
        json!({ "id": "1", "author_id": "43" }),
        None,
    );
    assert_eq!(
        interactions_from_activity(&repost, &bot()),
        vec![ActivityInteraction {
            user_id: "43".into(),
            username: None,
            kind: InteractionKind::Repost
        }]
    );
    let follow = event(
        "follow.follow",
        json!({ "source": { "id": "44" }, "target": { "id": BOT } }),
        None,
    );
    assert_eq!(
        interactions_from_activity(&follow, &bot()),
        vec![ActivityInteraction {
            user_id: "44".into(),
            username: None,
            kind: InteractionKind::Follow
        }]
    );
    let like = event(
        "like.create",
        json!({ "liked_tweet_id": "9", "tweet_author_id": BOT }),
        Some(json!({ "users": [{ "id": BOT }, { "id": "45", "username": "l" }] })),
    );
    assert_eq!(
        interactions_from_activity(&like, &bot()),
        vec![ActivityInteraction {
            user_id: "45".into(),
            username: Some("l".into()),
            kind: InteractionKind::Like
        }]
    );
    assert!(
        interactions_from_activity(
            &event(
                "post.mention.create",
                json!({ "id": "1", "author_id": BOT }),
                None
            ),
            &bot()
        )
        .is_empty()
    );
    assert!(
        interactions_from_activity(
            &event(
                "profile.update.bio",
                json!({ "before": "a", "after": "b" }),
                None
            ),
            &bot()
        )
        .is_empty()
    );
    assert!(interactions_from_activity(&json!({ "tweet_create_events": [] }), &bot()).is_empty());
}

#[test]
fn reads_the_envelope_and_its_event_id() {
    let parsed = activity_event(&event("like.create", json!({ "id": "1" }), None)).unwrap();
    assert_eq!(parsed.event_type, "like.create");
    assert_eq!(parsed.event_id.as_deref(), Some("777"));
    assert!(activity_event(&json!({ "data": [] })).is_none());
    assert!(activity_event(&json!({ "data": { "payload": {} } })).is_none());
}
