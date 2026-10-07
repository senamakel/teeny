use serde_json::json;

use super::*;

fn envelope(value: serde_json::Value) -> Envelope<Vec<RawPost>> {
    serde_json::from_value(value).unwrap()
}

#[test]
fn joins_authors_and_media_from_includes() {
    let response = envelope(json!({
        "data": [{
            "id": "1", "text": "gm", "author_id": "42", "conversation_id": "1",
            "created_at": "2026-09-14T09:40:00.000Z", "in_reply_to_user_id": "7",
            "attachments": { "media_keys": ["3_1", "3_missing"] },
            "public_metrics": { "like_count": 2, "reply_count": 1, "retweet_count": 3, "quote_count": 0, "impression_count": 99 }
        }],
        "includes": {
            "users": [{ "id": "42", "username": "someone" }],
            "media": [{ "media_key": "3_1", "type": "photo", "url": "https://pbs.twimg.com/a.jpg", "alt_text": "a cat" }]
        }
    }));
    let posts = to_posts(
        response.data.unwrap_or_default(),
        response.includes.as_ref(),
    );
    assert_eq!(
        serde_json::to_value(&posts).unwrap(),
        json!([{
            "id": "1", "text": "gm", "authorId": "42", "authorUsername": "someone", "conversationId": "1",
            "createdAt": "2026-09-14T09:40:00.000Z", "inReplyToUserId": "7",
            "media": [{ "type": "photo", "url": "https://pbs.twimg.com/a.jpg", "altText": "a cat" }],
            "metrics": { "id": "1", "likes": 2, "replies": 1, "reposts": 3, "quotes": 0, "impressions": 99 }
        }])
    );
}

#[test]
fn drops_posts_without_an_id_or_text_and_omits_what_x_left_out() {
    let response = envelope(json!({
        "data": [{ "id": "1" }, { "text": "orphan" }, { "id": "2", "text": "bare" }]
    }));
    let posts = to_posts(
        response.data.unwrap_or_default(),
        response.includes.as_ref(),
    );
    assert_eq!(
        serde_json::to_value(&posts).unwrap(),
        json!([{ "id": "2", "text": "bare" }])
    );
}

#[test]
fn a_post_round_trips_through_its_cached_form() {
    let post = XPost {
        author_id: Some("9".into()),
        ..XPost::new("5", "hi")
    };
    let back: XPost = serde_json::from_value(serde_json::to_value(&post).unwrap()).unwrap();
    assert_eq!(back, post);
}
