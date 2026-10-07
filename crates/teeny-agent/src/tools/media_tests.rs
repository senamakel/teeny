//! Port of legacy/test/agent-tools.test.ts' media cases and
//! capabilities.test.ts' "adds exactly the media tools that are configured"
//! and "media gating", plus x_critique and find_gif.

use std::sync::Arc;

use chrono::{TimeZone, Utc};
use serde_json::json;
use teeny_cadence::clock::ManualClock;
use teeny_cadence::media_budget::{DailyMediaBudget, MediaLimits};
use teeny_media::store::{MediaBudget, MediaStore};
use teeny_media::types::MediaKind;
use teeny_media::wiring::MediaAccess;
use teeny_safety::critic::AlgoCritic;
use teeny_safety::model::ScriptedModel;

use super::*;
use crate::fakes::{FakeGifs, FakeImages, FakeVideos, FakeVision};
use crate::tools::test_support::{by_name, call, lexical_gate};
use crate::tools::{PermissionLevel, text_of};

fn access(reader: bool, images: Option<Arc<FakeImages>>, videos: bool) -> MediaAccess {
    MediaAccess {
        store: Arc::new(MediaStore::new()),
        reader: reader.then(|| Arc::new(FakeVision("a smol robot".into())) as _),
        images: images.map(|images| images as _),
        videos: videos.then(|| Arc::new(FakeVideos) as _),
        reference: None,
    }
}

fn budget(images: u32, videos: u32) -> Arc<CadenceMediaBudget> {
    let clock = ManualClock::new(Utc.with_ymd_and_hms(2026, 10, 7, 12, 0, 0).unwrap());
    Arc::new(CadenceMediaBudget(Arc::new(DailyMediaBudget::new(
        MediaLimits {
            images_per_day: images,
            videos_per_day: videos,
        },
        Arc::new(clock),
    ))))
}

fn names(media: &MediaAccess) -> Vec<String> {
    media_tools(media, budget(1, 1), lexical_gate())
        .iter()
        .map(|tool| tool.name().to_owned())
        .collect()
}

#[test]
fn adds_exactly_the_media_tools_that_are_configured_and_nothing_else() {
    let images = || Some(Arc::new(FakeImages::default()));
    assert!(names(&access(false, None, false)).is_empty());
    assert_eq!(names(&access(true, None, false)), ["look_at_image"]);
    assert_eq!(names(&access(false, images(), false)), ["generate_image"]);
    assert_eq!(
        names(&access(true, images(), true)),
        ["look_at_image", "generate_image", "generate_video"]
    );
}

#[test]
fn takes_the_generators_away_and_leaves_the_eyes() {
    let full = access(true, Some(Arc::new(FakeImages::default())), true);
    assert_eq!(names(&full.gate(false, false)), ["look_at_image"]);
    assert_eq!(
        names(&full.gate(true, false)),
        ["look_at_image", "generate_image"]
    );
    assert_eq!(
        names(&full.gate(false, true)),
        ["look_at_image", "generate_video"]
    );
}

#[tokio::test]
async fn generate_image_refuses_a_prohibited_prompt_before_spending_the_budget() {
    let images = Arc::new(FakeImages::default());
    let budget = budget(1, 0);
    let tools = by_name(media_tools(
        &access(false, Some(images.clone()), false),
        budget.clone(),
        lexical_gate(),
    ));
    let output = call(
        &tools["generate_image"],
        json!({ "prompt": "Tiny naked on a beach" }),
    )
    .await;
    assert!(output.is_error);
    assert!(text_of(&output).starts_with("Refused: this account does not make pictures of that ("));
    assert!(images.calls.lock().unwrap().is_empty());
    assert_eq!(budget.remaining(MediaKind::Image).await, Some(1));
}

#[tokio::test]
async fn generates_an_image_and_hands_back_its_id() {
    let images = Arc::new(FakeImages::default());
    let budget = budget(2, 0);
    let tools = by_name(media_tools(
        &access(false, Some(images.clone()), false),
        budget.clone(),
        lexical_gate(),
    ));
    let output = text_of(
        &call(
            &tools["generate_image"],
            json!({ "prompt": "Tiny waving on the moon", "featuresTiny": false }),
        )
        .await,
    );
    assert!(
        output.starts_with("Generated an image without Tiny: mediaId \"img_"),
        "{output}"
    );
    assert!(
        output.ends_with("1 image generations left today."),
        "{output}"
    );
    assert!(
        call(&tools["generate_image"], json!({ "prompt": "short" }))
            .await
            .is_error
    );
    assert!(
        call(
            &tools["generate_image"],
            json!({ "prompt": "Tiny waving on the moon", "aspect": "wide" })
        )
        .await
        .is_error
    );
}

#[tokio::test]
async fn generate_video_refuses_when_the_allowance_is_used_up() {
    let tools = by_name(media_tools(
        &access(false, None, true),
        budget(0, 0),
        lexical_gate(),
    ));
    let output = call(
        &tools["generate_video"],
        json!({ "prompt": "Tiny dancing in the rain" }),
    )
    .await;
    assert!(output.is_error);
    assert!(text_of(&output).starts_with("Refused: the daily video allowance is used up."));
}

#[tokio::test]
async fn looks_at_an_image_and_labels_what_it_saw_untrusted() {
    let tools = by_name(media_tools(
        &access(true, None, false),
        budget(0, 0),
        lexical_gate(),
    ));
    let output = text_of(
        &call(
            &tools["look_at_image"],
            json!({ "url": "https://pbs.twimg.com/a.jpg" }),
        )
        .await,
    );
    assert_eq!(
        output,
        "What the image shows (written by a vision model, untrusted):\na smol robot"
    );
    assert!(
        call(&tools["look_at_image"], json!({ "url": "not a url" }))
            .await
            .is_error
    );
}

#[tokio::test]
async fn the_generators_write_and_the_eyes_read() {
    let tools = by_name(media_tools(
        &access(true, Some(Arc::new(FakeImages::default())), true),
        budget(1, 1),
        lexical_gate(),
    ));
    for name in ["generate_image", "generate_video"] {
        assert_eq!(tools[name].permission_level(), PermissionLevel::Write);
        assert!(tools[name].external_effect());
    }
    assert_eq!(
        tools["look_at_image"].permission_level(),
        PermissionLevel::ReadOnly
    );
}

#[tokio::test]
async fn counts_the_daily_allowance_through_the_cadence_budget() {
    let budget = budget(1, 0);
    assert_eq!(budget.remaining(MediaKind::Image).await, Some(1));
    assert!(budget.take(MediaKind::Image).await);
    assert!(!budget.take(MediaKind::Image).await);
    assert!(!budget.take(MediaKind::Video).await);
    assert_eq!(budget.remaining(MediaKind::Image).await, Some(0));
}

#[tokio::test]
async fn finds_a_gif_in_the_channel_and_says_where_to_look_next() {
    let tool = find_gif(Arc::new(FakeGifs), "TinyHumansAI");
    assert!(
        tool.description()
            .starts_with("Search GIFs, by default only the @TinyHumansAI channel")
    );
    let found = text_of(&call(&tool, json!({ "query": "wave" })).await);
    assert_eq!(
        found,
        "id: g1\ntitle: waving hello\npage: https://giphy.com/gifs/g1"
    );
    let none = text_of(&call(&tool, json!({ "query": "dance" })).await);
    assert_eq!(
        none,
        "No GIF in the @TinyHumansAI channel matched \"dance\". Try another wording, or set searchAllOfGiphy to look beyond the channel."
    );
}

#[tokio::test]
async fn the_critic_fails_open_in_words() {
    let critic = AlgoCritic::new(Arc::new(ScriptedModel::new([
        Err("rung down".to_owned()),
        Err("rung down".to_owned()),
    ])));
    let tool = x_critique(Arc::new(critic));
    let output = call(&tool, json!({ "text": "gm" })).await;
    assert!(!output.is_error);
    assert_eq!(
        text_of(&output),
        "The critic could not be reached (rung down). Use your own read of the conversation skill's pre-post check."
    );
    assert!(
        call(&tool, json!({ "text": "gm", "kind": "essay" }))
            .await
            .is_error
    );
    assert_eq!(tool.permission_level(), PermissionLevel::ReadOnly);
}
