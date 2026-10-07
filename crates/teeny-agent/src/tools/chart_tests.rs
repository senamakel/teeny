//! Port of legacy/test/market-tools.test.ts' render_chart cases. The
//! post_chart cases moved with the posting to the chart job
//! (`teeny-jobs` `chart/mod_tests.rs`): the agent renders, the host posts.

use std::sync::Arc;

use serde_json::json;

use super::*;
use crate::fakes::{FakeMarket, TOKEN};
use crate::tools::test_support::call;
use crate::tools::{PermissionLevel, text_of};

fn render(book: &Arc<ChartBook>, bars: usize) -> Arc<dyn Tool> {
    render_chart(
        ChartSource {
            market: Arc::new(FakeMarket { bars }),
            address: TOKEN.into(),
            symbol: Some("TINY".into()),
            watermark: Some("@teenydroid".into()),
            output_dir: None,
        },
        Arc::clone(book),
    )
}

#[tokio::test]
async fn hands_back_a_chart_id_rather_than_image_bytes() {
    let book = Arc::new(ChartBook::default());
    let output = text_of(&call(&render(&book, 60), json!({ "timeframe": "1h", "bars": 60 })).await);
    assert!(
        output.starts_with("Rendered chart_1: 1h, 60 bars, 1200x675px, "),
        "{output}"
    );
    assert!(
        output.ends_with("Put chartId \"chart_1\" in your answer to attach it."),
        "{output}"
    );
    let too_few = text_of(&call(&render(&book, 4), json!({})).await);
    assert_eq!(too_few, "Only 4 bars available; not enough to chart.");
}

#[tokio::test]
async fn the_book_hands_the_host_the_chart_the_agent_named() {
    let book = Arc::new(ChartBook::default());
    call(&render(&book, 60), json!({ "timeframe": "1h", "bars": 60 })).await;
    let chart = book.get("chart_1").expect("rendered");
    assert!(chart.png.starts_with(b"\x89PNG"));
    for unknown in ["chart_2", "chart_0", "chart_x", "1"] {
        assert!(book.get(unknown).is_none(), "{unknown}");
    }
}

#[test]
fn render_chart_reads() {
    let book = Arc::new(ChartBook::default());
    let tool = render(&book, 60);
    assert_eq!(tool.permission_level(), PermissionLevel::ReadOnly);
    assert!(!tool.external_effect());
}
