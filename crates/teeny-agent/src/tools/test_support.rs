//! Fakes shared by the tool tests.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::Value;
use teeny_core::types::{PostId, UserId};
use teeny_x::cache::XReads;
use teeny_x::model::{OwnPostMetrics, SearchOptions, Trend, TrendOptions, XPost, XUser};
use teeny_x::{XError, XResult};

use super::{Tool, ToolResult};

/// Runs a tool.
pub async fn call(tool: &Arc<dyn Tool>, args: Value) -> ToolResult {
    tool.execute(args).await.expect("the tool ran")
}

/// The tools by name.
pub fn by_name(tools: Vec<Arc<dyn Tool>>) -> HashMap<String, Arc<dyn Tool>> {
    tools
        .into_iter()
        .map(|tool| (tool.name().to_owned(), tool))
        .collect()
}

/// X reads answered from memory, remembering what was asked.
#[derive(Default)]
pub struct FakeReads {
    pub posts: Vec<XPost>,
    pub user: Option<XUser>,
    pub searches: Mutex<Vec<(String, SearchOptions)>>,
    pub conversations: Mutex<Vec<(PostId, Option<u32>)>>,
}

#[async_trait]
impl XReads for FakeReads {
    async fn search_posts(&self, query: &str, options: &SearchOptions) -> XResult<Vec<XPost>> {
        self.searches
            .lock()
            .unwrap()
            .push((query.to_owned(), options.clone()));
        Ok(self.posts.clone())
    }
    async fn get_post(&self, id: &PostId) -> XResult<Option<XPost>> {
        Ok(self.posts.iter().find(|post| &post.id == id).cloned())
    }
    async fn get_conversation(&self, id: &PostId, max: Option<u32>) -> XResult<Vec<XPost>> {
        self.conversations.lock().unwrap().push((id.clone(), max));
        Ok(self.posts.clone())
    }
    async fn trends(&self, _options: TrendOptions) -> XResult<Vec<Trend>> {
        Ok(Vec::new())
    }
    async fn post_metrics(&self, _ids: &[PostId]) -> XResult<Vec<OwnPostMetrics>> {
        Ok(Vec::new())
    }
    async fn follows_account(&self, _user_id: &UserId) -> Option<bool> {
        None
    }
    async fn user_by_username(&self, username: &str) -> XResult<Option<XUser>> {
        if username == "broken" {
            return Err(XError::Http {
                status: 503,
                body: "down".into(),
            });
        }
        Ok(self.user.clone())
    }
}

/// The post the legacy agent-tools test used.
pub fn post() -> XPost {
    XPost {
        author_username: Some("someone".into()),
        conversation_id: Some("9".into()),
        ..XPost::new("10", "hello @stonksrocks")
    }
}

/// A client on a mock X, with an app bearer and an OAuth pair.
pub fn x_api(server: &wiremock::MockServer) -> Arc<teeny_x::XApi> {
    use teeny_core::Secret;
    Arc::new(
        teeny_x::XApi::new(
            "teenydroid",
            teeny_x::oauth::OAuth1::new("ck", Secret::new("cs"), "at", Secret::new("ats")),
            Some(Secret::new("app-bearer")),
        )
        .with_base_url(server.uri()),
    )
}

/// X takes any upload and calls it `media-1`.
pub async fn accept_uploads(server: &wiremock::MockServer) {
    use wiremock::matchers::{method, path, path_regex};
    use wiremock::{Mock, ResponseTemplate};
    Mock::given(method("POST"))
        .and(path("/2/media/upload/initialize"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "data": { "id": "media-1" } })),
        )
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path_regex("^/2/media/upload/media-1/(append|finalize)$"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "data": { "id": "media-1" } })),
        )
        .mount(server)
        .await;
}

/// X accepts exactly `times` posts with this body, and calls each `id`.
pub async fn expect_post(server: &wiremock::MockServer, body: Value, id: &str, times: u64) {
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, ResponseTemplate};
    Mock::given(method("POST"))
        .and(path("/2/tweets"))
        .and(body_json(body))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(serde_json::json!({ "data": { "id": id } })),
        )
        .expect(times)
        .mount(server)
        .await;
}

/// Nothing reaches X's post endpoint.
pub async fn expect_no_posts(server: &wiremock::MockServer) {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, ResponseTemplate};
    Mock::given(method("POST"))
        .and(path("/2/tweets"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(server)
        .await;
}

/// The content filter with no judge: the lexicon and the link allowlist.
pub fn lexical_gate() -> Arc<teeny_safety::gate::SafetyGate> {
    Arc::new(teeny_safety::gate::SafetyGate::new(
        teeny_safety::gate::SafetyGateOptions::default(),
    ))
}
