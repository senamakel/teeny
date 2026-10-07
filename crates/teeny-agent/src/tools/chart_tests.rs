//! Port of legacy/test/market-tools.test.ts' render_chart and post_chart
//! cases, now through the XPublisher guard.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;
use teeny_memory::{Content, Experience, InMemoryMemory, Memory, MemoryError, Person};
use teeny_store::holds::HoldSurface;
use teeny_x::XPublisher;
use wiremock::MockServer;

use super::*;
use crate::fakes::{FakeMarket, TOKEN};
use crate::tools::post::PostingGuard;
use crate::tools::test_support::{
    accept_uploads, call, expect_no_posts, expect_post, lexical_gate, x_api,
};
use crate::tools::{PermissionLevel, text_of};

struct Setup {
    server: MockServer,
    book: Arc<ChartBook>,
    memory: Arc<InMemoryMemory>,
}

impl Setup {
    async fn new() -> Self {
        Self {
            server: MockServer::start().await,
            book: Arc::new(ChartBook::default()),
            memory: Arc::new(InMemoryMemory::default()),
        }
    }

    fn render(&self, bars: usize) -> Arc<dyn Tool> {
        render_chart(
            ChartSource {
                market: Arc::new(FakeMarket { bars }),
                address: TOKEN.into(),
                symbol: Some("TINY".into()),
                watermark: Some("@teenydroid".into()),
                output_dir: None,
            },
            self.book.clone(),
        )
    }

    fn post(&self, dry_run: bool, memory: Arc<dyn Memory>) -> Arc<dyn Tool> {
        let x = x_api(&self.server);
        let guard = PostingGuard::new(lexical_gate(), HoldSurface::Chart, "post_chart", 1);
        post_chart(PostChartTool {
            publisher: Arc::new(XPublisher::new(x.clone(), Arc::new(guard), dry_run)),
            x,
            book: self.book.clone(),
            memory: Some(memory),
            symbol: "TINY".into(),
            pacing: None,
        })
    }

    fn chart_posts(&self) -> Vec<serde_json::Value> {
        self.memory
            .experiences()
            .into_iter()
            .filter_map(|experience| match experience.content {
                Content::Json { data } if data["kind"] == "chart_post" => {
                    Some(data["payload"].clone())
                }
                _ => None,
            })
            .collect()
    }
}

#[tokio::test]
async fn hands_back_a_chart_id_rather_than_image_bytes() {
    let s = Setup::new().await;
    let output = text_of(&call(&s.render(60), json!({ "timeframe": "1h", "bars": 60 })).await);
    assert!(
        output.starts_with("Rendered chart_1: 1h, 60 bars, 1200x675px, "),
        "{output}"
    );
    assert!(
        output.ends_with("Pass chartId \"chart_1\" to post_chart to attach it."),
        "{output}"
    );
    let too_few = text_of(&call(&s.render(4), json!({})).await);
    assert_eq!(too_few, "Only 4 bars available; not enough to chart.");
}

#[tokio::test]
async fn attaches_a_rendered_chart_to_the_post_it_publishes_and_remembers_it() {
    let s = Setup::new().await;
    accept_uploads(&s.server).await;
    expect_post(
        &s.server,
        json!({ "text": "the level held", "media": { "media_ids": ["media-1"] } }),
        "999",
        1,
    )
    .await;
    call(&s.render(60), json!({ "timeframe": "1h", "bars": 60 })).await;
    let output = text_of(
        &call(
            &s.post(false, s.memory.clone()),
            json!({ "text": "the level held", "chartId": "chart_1" }),
        )
        .await,
    );
    assert_eq!(output, "Published post 999 with the chart attached.");
    assert_eq!(
        s.chart_posts(),
        [
            json!({ "text": "the level held", "symbol": "TINY", "chart": "chart_1", "published": true })
        ]
    );
}

#[tokio::test]
async fn cuts_a_closing_question_off_the_post_rather_than_arguing_with_the_agent() {
    let s = Setup::new().await;
    accept_uploads(&s.server).await;
    expect_post(
        &s.server,
        json!({ "text": "the level held. we are still here.", "media": { "media_ids": ["media-1"] } }),
        "999",
        1,
    )
    .await;
    call(&s.render(60), json!({})).await;
    call(
        &s.post(false, s.memory.clone()),
        json!({ "text": "the level held. we are still here.\nwhere do you see the next base?", "chartId": "chart_1" }),
    )
    .await;
}

#[tokio::test]
async fn refuses_an_id_it_never_rendered_instead_of_posting_a_bare_text() {
    let s = Setup::new().await;
    expect_no_posts(&s.server).await;
    let output = call(
        &s.post(false, s.memory.clone()),
        json!({ "text": "hello", "chartId": "chart_9" }),
    )
    .await;
    assert!(output.is_error);
    assert_eq!(
        text_of(&output),
        "Refused: no chart with id chart_9. Call render_chart and use the id it returns."
    );
}

#[tokio::test]
async fn publishes_nothing_in_a_dry_run_and_records_the_rehearsal_as_one() {
    let s = Setup::new().await;
    expect_no_posts(&s.server).await;
    let output = text_of(&call(&s.post(true, s.memory.clone()), json!({ "text": "draft" })).await);
    assert_eq!(
        output,
        "DRY RUN, nothing was published. Would have posted: {\"text\":\"draft\"}"
    );
    assert_eq!(
        s.chart_posts(),
        [json!({ "text": "draft", "symbol": "TINY", "published": false })]
    );
}

struct Down;
#[async_trait]
impl Memory for Down {
    async fn capture(&self, _experience: Experience) -> teeny_memory::Result<()> {
        Err(MemoryError::InvalidInput("401 MISSING_TOKEN".into()))
    }
    async fn recall(&self, _query: &str, _person: Option<&Person>) -> teeny_memory::Result<String> {
        Ok(String::new())
    }
}

#[tokio::test]
async fn does_not_report_a_memory_outage_as_a_failed_post() {
    let s = Setup::new().await;
    expect_post(&s.server, json!({ "text": "the level held" }), "999", 1).await;
    let output = call(
        &s.post(false, Arc::new(Down)),
        json!({ "text": "the level held" }),
    )
    .await;
    assert!(!output.is_error);
    assert!(text_of(&output).contains("Published post 999"));
}

#[tokio::test]
async fn enforces_the_emoji_cap_and_the_content_filter() {
    let s = Setup::new().await;
    expect_no_posts(&s.server).await;
    let tool = s.post(false, s.memory.clone());
    let emoji = call(&tool, json!({ "text": "up only 🚀🚀🚀" })).await;
    assert!(text_of(&emoji).starts_with("Refused"));
    let held = call(&tool, json!({ "text": "the chart is fucking green" })).await;
    assert!(text_of(&held).contains("Refused"));
    let linked = call(&tool, json!({ "text": "chart at https://evil.xyz/x" })).await;
    assert!(text_of(&linked).contains("not on the allowlist"));
    assert!(s.chart_posts().is_empty());
}

#[tokio::test]
async fn post_chart_writes_and_render_chart_reads() {
    let s = Setup::new().await;
    let post = s.post(true, s.memory.clone());
    assert_eq!(post.permission_level(), PermissionLevel::Write);
    assert!(post.external_effect());
    assert_eq!(s.render(60).permission_level(), PermissionLevel::ReadOnly);
}
