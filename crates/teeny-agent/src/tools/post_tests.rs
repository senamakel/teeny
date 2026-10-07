//! Port of legacy/test/agent-tools.test.ts' x_post cases ("X tools", "emoji
//! limit", "content filter on the publishing tools"), now through the
//! XPublisher guard, plus the cadence and the reply budget the guard adds.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use serde_json::json;
use teeny_cadence::budget::{BudgetRules, RepliedCounter, ReplyBudget};
use teeny_cadence::cadence::{Cadence, CadenceRules};
use teeny_cadence::clock::{SystemClock, iso};
use teeny_media::store::{MediaStore, NewMedia};
use teeny_media::types::{MediaKind, MimeType};
use teeny_store::holds::HoldSurface;
use teeny_store::mentions::RepliedFilter;
use teeny_store::posts::{InMemoryPostLedger, NewOwnPost, PostLedger};
use teeny_x::XPublisher;
use teeny_x::model::XPost;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use super::*;
use crate::tools::test_support::{
    FakeReads, accept_uploads, call, expect_no_posts, expect_post, lexical_gate, x_api,
};
use crate::tools::{PermissionLevel, text_of};

struct Setup {
    server: MockServer,
    media: Arc<MediaStore>,
    ledger: Arc<InMemoryPostLedger>,
}

impl Setup {
    async fn new() -> Self {
        Self {
            server: MockServer::start().await,
            media: Arc::new(MediaStore::new()),
            ledger: Arc::new(InMemoryPostLedger::default()),
        }
    }

    fn pacing(&self, replies: Option<Arc<dyn ReplyAllowance>>) -> Arc<Pacing> {
        let cadence = Cadence::new(
            self.ledger.clone(),
            CadenceRules {
                min_gap: chrono::TimeDelta::hours(6),
                tolerance: chrono::TimeDelta::minutes(20),
            },
            Arc::new(SystemClock),
        );
        let pacing = Pacing::new(Arc::new(cadence), "agent");
        Arc::new(match replies {
            Some(replies) => pacing.with_replies(replies),
            None => pacing,
        })
    }

    fn tool(&self, dry_run: bool, pacing: Option<Arc<Pacing>>) -> Arc<dyn Tool> {
        let x = x_api(&self.server);
        let mut guard = PostingGuard::new(lexical_gate(), HoldSurface::X, "x_post", 1);
        if let Some(pacing) = &pacing {
            guard = guard.with_pacing(Arc::clone(pacing));
        }
        x_post(XPostTool {
            publisher: Arc::new(XPublisher::new(x.clone(), Arc::new(guard), dry_run)),
            x,
            media: Some(self.media.clone()),
            gifs: None,
            pacing,
            max_emoji: 1,
        })
    }
}

#[tokio::test]
async fn does_not_publish_in_dry_run_mode() {
    let s = Setup::new().await;
    expect_no_posts(&s.server).await;
    let output = call(&s.tool(true, None), json!({ "text": "gm" })).await;
    assert_eq!(
        text_of(&output),
        "DRY RUN, nothing was published. Would have posted: {\"text\":\"gm\"}"
    );
}

#[tokio::test]
async fn publishes_replies_when_dry_run_is_off() {
    let s = Setup::new().await;
    expect_post(
        &s.server,
        json!({ "text": "gm", "reply": { "in_reply_to_tweet_id": "10" } }),
        "77",
        1,
    )
    .await;
    let output = call(
        &s.tool(false, None),
        json!({ "text": "gm", "replyToPostId": "10" }),
    )
    .await;
    assert_eq!(text_of(&output), "Published post 77.");
}

#[tokio::test]
async fn attaches_a_generated_file_by_its_id_and_refuses_an_id_it_does_not_hold() {
    let s = Setup::new().await;
    accept_uploads(&s.server).await;
    expect_post(
        &s.server,
        json!({ "text": "gm", "media": { "media_ids": ["media-1"] } }),
        "77",
        1,
    )
    .await;
    let stored = s.media.put(NewMedia {
        kind: MediaKind::Image,
        data: vec![1],
        mime_type: MimeType::Png,
        prompt: "p".into(),
    });
    let tool = s.tool(false, None);
    let output = call(&tool, json!({ "text": "gm", "mediaId": stored.id })).await;
    assert_eq!(
        text_of(&output),
        "Published post 77 with a generated image."
    );

    let refused = call(&tool, json!({ "text": "gm", "mediaId": "img_madeup_1" })).await;
    assert!(refused.is_error);
    assert!(text_of(&refused).starts_with("Refused"));
}

#[tokio::test]
async fn refuses_a_gif_without_a_source_and_a_gif_with_a_file() {
    let s = Setup::new().await;
    expect_no_posts(&s.server).await;
    let tool = s.tool(false, None);
    let no_source = call(&tool, json!({ "text": "gm", "gifId": "abc" })).await;
    assert_eq!(
        text_of(&no_source),
        "Refused: no GIF source is configured, so gifId cannot be attached."
    );
    assert!(
        call(&tool, json!({ "text": "gm", "mediaId": "not-an-id" }))
            .await
            .is_error
    );
}

#[tokio::test]
async fn refuses_a_post_with_more_emoji_than_the_account_allows() {
    let s = Setup::new().await;
    expect_no_posts(&s.server).await;
    let output = call(&s.tool(false, None), json!({ "text": "gm frens 🛠️ 🚀" })).await;
    assert!(output.is_error);
    assert!(text_of(&output).starts_with("Refused"));
}

#[tokio::test]
async fn refuses_a_draft_the_filter_holds_and_says_not_to_rephrase() {
    let s = Setup::new().await;
    expect_no_posts(&s.server).await;
    let output = call(
        &s.tool(false, None),
        json!({ "text": "this is fucking great" }),
    )
    .await;
    let text = text_of(&output);
    assert!(output.is_error);
    assert!(text.contains("Refused"), "{text}");
    assert!(text.contains("profanity"), "{text}");
    assert!(text.contains("Do not rephrase"), "{text}");
}

#[tokio::test]
async fn refuses_a_link_off_the_allowlist_rather_than_publishing_a_gap() {
    let s = Setup::new().await;
    expect_no_posts(&s.server).await;
    let output = call(
        &s.tool(false, None),
        json!({ "text": "claim at https://claim-tiny.xyz/go" }),
    )
    .await;
    assert!(text_of(&output).contains("not on the allowlist"));
}

#[tokio::test]
async fn publishes_a_clean_draft_with_an_allowlisted_link() {
    let s = Setup::new().await;
    expect_post(
        &s.server,
        json!({ "text": "code at github.com/tinyhumansai" }),
        "78",
        1,
    )
    .await;
    let output = call(
        &s.tool(false, None),
        json!({ "text": "code at github.com/tinyhumansai" }),
    )
    .await;
    assert_eq!(text_of(&output), "Published post 78.");
}

#[tokio::test]
async fn holds_a_standalone_post_to_the_gap_and_records_one_that_goes_out() {
    let s = Setup::new().await;
    expect_post(&s.server, json!({ "text": "gm" }), "80", 1).await;
    let tool = s.tool(false, Some(s.pacing(None)));
    assert_eq!(
        text_of(&call(&tool, json!({ "text": "gm" })).await),
        "Published post 80."
    );
    let recorded = s.ledger.latest().await.unwrap().unwrap();
    assert_eq!(
        (recorded.post_id.as_str(), recorded.kind.as_str()),
        ("80", "agent")
    );
    assert_eq!(recorded.text.as_deref(), Some("gm"));

    let held = call(&tool, json!({ "text": "gm again" })).await;
    assert!(held.is_error);
    assert!(
        text_of(&held).starts_with("Refused: cadence: too soon after post 80"),
        "{}",
        text_of(&held)
    );
}

#[tokio::test]
async fn a_reply_is_not_held_to_the_gap() {
    let s = Setup::new().await;
    s.ledger
        .record(&NewOwnPost {
            post_id: "1".into(),
            kind: "morning".into(),
            posted_at: iso(Utc::now()),
            text: None,
        })
        .await
        .unwrap();
    expect_post(
        &s.server,
        json!({ "text": "gm", "reply": { "in_reply_to_tweet_id": "10" } }),
        "81",
        1,
    )
    .await;
    let output = call(
        &s.tool(false, Some(s.pacing(None))),
        json!({ "text": "gm", "replyToPostId": "10" }),
    )
    .await;
    assert_eq!(text_of(&output), "Published post 81.");
}

struct Replied(u64);
#[async_trait]
impl RepliedCounter for Replied {
    async fn count_replied(&self, _filter: &RepliedFilter) -> teeny_store::Result<u64> {
        Ok(self.0)
    }
}

#[tokio::test]
async fn holds_a_reply_to_the_reply_budget() {
    let s = Setup::new().await;
    Mock::given(method("GET"))
        .and(path("/2/users/me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "id": "999", "username": "teenydroid" }
        })))
        .mount(&s.server)
        .await;
    expect_no_posts(&s.server).await;
    let stranger = XPost {
        conversation_id: Some("10".into()),
        ..XPost::new("10", "a stranger talking")
    };
    let budget = ReplyBudget::new(
        s.ledger.clone(),
        Arc::new(Replied(10)),
        BudgetRules {
            auto_per_post: 10,
            smart_per_post: 10,
            gap: chrono::TimeDelta::hours(6),
            bot_username: Some("teenydroid".into()),
            thread_window: chrono::TimeDelta::days(7),
        },
        Arc::new(SystemClock),
    );
    let allowance = BudgetAllowance::new(
        Arc::new(FakeReads {
            posts: vec![stranger],
            ..FakeReads::default()
        }),
        Arc::new(budget),
        x_api(&s.server),
    );
    let tool = s.tool(false, Some(s.pacing(Some(Arc::new(allowance)))));
    let held = call(&tool, json!({ "text": "gm", "replyToPostId": "10" })).await;
    assert!(held.is_error);
    assert_eq!(
        text_of(&held),
        "Refused: the smart reply budget is used up (10 of 10). Nothing was published; do not reply again until it has room."
    );
}

#[tokio::test]
async fn uploads_nothing_for_a_refused_post() {
    let s = Setup::new().await;
    expect_no_posts(&s.server).await;
    Mock::given(|request: &Request| request.url.path().starts_with("/2/media"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&s.server)
        .await;
    let stored = s.media.put(NewMedia {
        kind: MediaKind::Image,
        data: vec![1],
        mime_type: MimeType::Png,
        prompt: "p".into(),
    });
    let output = call(
        &s.tool(false, None),
        json!({ "text": "this is fucking great", "mediaId": stored.id }),
    )
    .await;
    assert!(output.is_error);
}

#[tokio::test]
async fn x_post_is_a_write_with_an_outside_effect() {
    let s = Setup::new().await;
    let tool = s.tool(true, None);
    assert_eq!(tool.permission_level(), PermissionLevel::Write);
    assert!(tool.external_effect());
    assert!(tool.description().ends_with("At most 1 emoji per post."));
}
