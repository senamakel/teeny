//! Port of legacy/test/agent-tools.test.ts, "X tools" (the reads), plus the
//! new `x_user`.

use std::sync::Arc;

use serde_json::json;
use teeny_x::model::{SortOrder, XPost, XPostMedia, XUser};

use super::*;
use crate::tools::test_support::{FakeReads, call, post};
use crate::tools::text_of;

fn reads(posts: Vec<XPost>) -> Arc<FakeReads> {
    Arc::new(FakeReads {
        posts,
        ..FakeReads::default()
    })
}

#[tokio::test]
async fn renders_search_results_with_the_ids_the_agent_needs_to_follow_up() {
    let x = reads(vec![post()]);
    let output = text_of(
        &call(
            &search_posts(x.clone(), 10),
            json!({ "query": "@stonksrocks" }),
        )
        .await,
    );
    assert!(output.contains("id: 10"));
    assert!(output.contains("conversation_id: 9"));
    assert!(output.contains("author: @someone"));
    let searches = x.searches.lock().unwrap();
    assert_eq!(searches[0].1.max_results, Some(10));
    assert_eq!(searches[0].1.sort_order, Some(SortOrder::Recency));
}

#[tokio::test]
async fn holds_a_search_to_the_configured_cap() {
    let x = reads(Vec::new());
    let tool = search_posts(x.clone(), 25);
    let output = call(
        &tool,
        json!({ "query": "q", "maxResults": 80, "sortOrder": "relevancy" }),
    )
    .await;
    assert_eq!(text_of(&output), "No posts matched.");
    {
        let searches = x.searches.lock().unwrap();
        assert_eq!(searches[0].1.max_results, Some(25));
        assert_eq!(searches[0].1.sort_order, Some(SortOrder::Relevancy));
    }
    assert!(
        call(&tool, json!({ "query": "q", "maxResults": 5 }))
            .await
            .is_error
    );
    assert!(call(&tool, json!({ "query": "" })).await.is_error);
}

#[tokio::test]
async fn lists_a_post_s_pictures_so_the_agent_can_look_at_them() {
    let looked = XPost {
        media: vec![
            XPostMedia {
                kind: "photo".into(),
                url: Some("https://pbs.twimg.com/media/a.jpg".into()),
                preview_url: None,
                alt_text: Some("a chart".into()),
            },
            XPostMedia {
                kind: "video".into(),
                url: None,
                preview_url: Some("https://pbs.twimg.com/media/b.jpg".into()),
                alt_text: None,
            },
        ],
        ..XPost::new("10", "look")
    };
    let output = text_of(&call(&get_post(reads(vec![looked])), json!({ "id": "10" })).await);
    assert!(output.contains("media: photo https://pbs.twimg.com/media/a.jpg (alt: a chart)"));
    assert!(output.contains("media: video https://pbs.twimg.com/media/b.jpg"));
    let missing = call(&get_post(reads(Vec::new())), json!({ "id": "11" })).await;
    assert_eq!(text_of(&missing), "No post found with id 11.");
    assert!(
        call(&get_post(reads(Vec::new())), json!({ "id": "abc" }))
            .await
            .is_error
    );
}

#[test]
fn renders_posts_as_the_legacy_tool_did() {
    let full = XPost {
        author_username: Some("a".into()),
        conversation_id: Some("1".into()),
        created_at: Some("2026-10-01T00:00:00.000Z".into()),
        ..XPost::new("2", "hi")
    };
    assert_eq!(
        render_posts(&[full, XPost::new("3", "yo")]),
        "id: 2\nauthor: @a\nconversation_id: 1\ncreated_at: 2026-10-01T00:00:00.000Z\ntext: hi\n---\nid: 3\ntext: yo"
    );
}

#[tokio::test]
async fn reads_a_conversation_under_the_cap() {
    let x = reads(vec![post()]);
    let output = call(
        &get_comments(x.clone(), 10),
        json!({ "conversationId": "9", "maxResults": 50 }),
    )
    .await;
    assert!(text_of(&output).contains("id: 10"));
    assert_eq!(x.conversations.lock().unwrap()[0], ("9".into(), Some(10)));
}

#[tokio::test]
async fn looks_an_account_up_by_handle() {
    let x = Arc::new(FakeReads {
        user: Some(XUser {
            id: "42".into(),
            username: "StonksRocks".into(),
            name: Some("Stonks".into()),
            description: Some("an open-source dapp".into()),
            created_at: Some("2021-03-04T05:06:07.000Z".into()),
            verified: Some(false),
            followers: Some(1200),
            following: Some(30),
            posts: Some(999),
        }),
        ..FakeReads::default()
    });
    let output = text_of(&call(&user(x), json!({ "username": "@stonksrocks" })).await);
    assert_eq!(
        output,
        "id: 42\nusername: @StonksRocks\nname: Stonks\nbio: an open-source dapp\njoined: 2021-03-04T05:06:07.000Z\nfollowers: 1200\nfollowing: 30\nposts: 999"
    );
    let nobody = call(&user(reads(Vec::new())), json!({ "username": "nobody" })).await;
    assert_eq!(
        text_of(&nobody),
        "No X account found with the handle @nobody."
    );
    let failed = call(&user(reads(Vec::new())), json!({ "username": "broken" })).await;
    assert!(failed.is_error);
    assert!(
        call(
            &user(reads(Vec::new())),
            json!({ "username": "not a handle" })
        )
        .await
        .is_error
    );
}
