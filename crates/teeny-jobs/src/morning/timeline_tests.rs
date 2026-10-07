use teeny_x::model::{Trend, XPostMedia, XPostMetrics};

use super::*;
use crate::morning::news::render_stories;
use crate::morning::test_support::FakeReads;

fn metrics(likes: u64, replies: u64, reposts: u64, quotes: u64, impressions: u64) -> XPostMetrics {
    XPostMetrics {
        id: "m".into(),
        likes,
        replies,
        reposts,
        quotes,
        impressions: Some(impressions),
        bookmarks: None,
    }
}

fn meme() -> XPost {
    XPost {
        author_username: Some("someone".into()),
        created_at: Some("2026-09-14T04:00:00Z".into()),
        media: vec![XPostMedia {
            kind: "photo".into(),
            url: Some("https://pbs/x.jpg".into()),
            preview_url: None,
            alt_text: Some("a robot nodding".into()),
        }],
        metrics: Some(metrics(4_200, 310, 90, 140, 900_000)),
        ..XPost::new(
            "77",
            "the model said \"you're absolutely right\" nine times in one session and i am cooked\nhttps://t.co/x",
        )
    }
}

fn quiet() -> XPost {
    XPost {
        author_username: Some("lab".into()),
        metrics: Some(metrics(12, 0, 1, 0, 0)),
        ..XPost::new("78", "new open weights model dropped, benchmarks look fine")
    }
}

#[test]
fn turns_a_post_into_a_story_with_its_counters_picture_and_url() {
    let story = story_of(&meme(), &default_queries()[0]);
    assert_eq!(
        story.title,
        "the model said \"you're absolutely right\" nine times in one session and i am cooked"
    );
    assert_eq!(story.url, "https://x.com/someone/status/77");
    assert_eq!(story.source, "@someone on X");
    assert_eq!(story.topic, "ai memes");
    assert_eq!(story.kind, StoryKind::X);
    assert_eq!(story.media.as_deref(), Some("a robot nodding"));
    assert!(!story.excerpt.contains("https://"));
}

#[test]
fn values_a_reply_as_ten_likes() {
    let queries = default_queries();
    assert_eq!(conversation_score(&story_of(&quiet(), &queries[1])), 12.0 * 0.5 + 1.0);
    assert_eq!(
        conversation_score(&story_of(&meme(), &queries[0])),
        310.0 * 5.0 + 140.0 * 5.0 + 90.0 + 4_200.0 * 0.5
    );
}

#[tokio::test]
async fn reads_recent_originals_by_relevancy_best_conversations_first() {
    let reads = std::sync::Arc::new(FakeReads::answering([Ok(vec![quiet(), meme()])]));
    let now = chrono::DateTime::parse_from_rfc3339("2026-09-14T08:00:00Z").unwrap().to_utc();
    let source = XTimelineSource::new(reads.clone(), None, now);
    let found = source.pulse(&default_queries()[0], 24, 20).await;
    let urls: Vec<_> = found.iter().map(|s| s.url.as_str()).collect();
    assert_eq!(urls, ["https://x.com/someone/status/77", "https://x.com/lab/status/78"]);
    let (query, options) = reads.searched()[0].clone();
    assert!(query.contains("has:media -has:cashtags -is:retweet -is:reply"));
    assert_eq!(options.max_results, Some(20));
    assert_eq!(options.start_time.as_deref(), Some("2026-09-13T07:59:00.000Z"));
    assert_eq!(options.end_time.as_deref(), Some("2026-09-14T07:59:00.000Z"));
    assert_eq!(options.sort_order, Some(SortOrder::Relevancy));
    assert!(source.trending().await.is_empty());

    let broken = std::sync::Arc::new(FakeReads::answering([Err(teeny_x::XError::Missing("429".into()))]));
    let source = XTimelineSource::new(broken, Some(1), now);
    assert!(source.pulse(&default_queries()[0], 24, 10).await.is_empty());
    assert!(source.trending().await.is_empty());
}

struct Corners;

#[async_trait::async_trait]
impl TimelineSource for Corners {
    async fn trending(&self) -> Vec<Trend> {
        vec![
            Trend { name: "gm".into(), posts: Some(100) },
            Trend { name: "OpenAI".into(), posts: Some(5_000) },
        ]
    }
    async fn pulse(&self, query: &TimelineQuery, _hours: u32, _limit: u32) -> Vec<Story> {
        let queries = default_queries();
        if query.label == "ai memes" {
            vec![
                story_of(&meme(), &queries[0]),
                story_of(&XPost::new("1", "AIRDROP live, dm me"), &queries[0]),
            ]
        } else {
            vec![
                story_of(&XPost::new("2", "the senate hearing on AI was wild"), &queries[1]),
                story_of(&quiet(), &queries[1]),
            ]
        }
    }
}

#[tokio::test]
async fn gathers_every_corner_drops_promotions_and_politics_and_is_off_at_zero() {
    let read = gather_timeline(&Corners, &default_queries(), 24, 10).await;
    let titles: Vec<_> = read.posts.iter().map(|s| s.title.clone()).collect();
    assert_eq!(titles, [meme().text.split('\n').next().unwrap().to_owned()]);
    assert_eq!(read.dropped, 2);
    assert_eq!(read.quiet, 1);
    assert_eq!(render_trends(&read.trends), "- OpenAI (5,000 posts)\n- gm (100 posts)");
    let off = gather_timeline(&Corners, &default_queries(), 24, 0).await;
    assert!(off.posts.is_empty() && off.trends.is_empty() && off.dropped == 0);
}

#[test]
fn a_contract_address_or_another_cashtag_is_a_promotion() {
    assert!(is_spam("hear me out, ca: 0x7927cd547e644db657d95cb04ea678c5feb37777 could run"));
    assert!(is_spam("$DARWIN is not an agent, it rewrites itself"));
    assert!(!is_spam("$TINY holders, the model said we are absolutely right again"));
}

#[test]
fn numbers_the_timeline_after_the_news_with_counters() {
    let rendered = render_stories(&[story_of(&meme(), &default_queries()[0])], 3);
    assert!(rendered.contains("[3] the model said"));
    assert!(rendered.contains("@someone on X, 2026-09-14 (ai memes)"));
    assert!(rendered.contains("picture: a robot nodding"));
    assert!(rendered.contains("310 replies, 140 quotes, 90 reposts, 4.2k likes, 900k views"));
}

#[test]
fn reads_corners_from_configuration_or_the_built_in_ones() {
    assert_eq!(parse_timeline_queries(""), default_queries());
    let parsed = parse_timeline_queries("robots=(robot OR robots) -is:retweet; memes");
    assert_eq!(parsed[0].label, "robots");
    assert_eq!(parsed[0].query, "(robot OR robots) -is:retweet");
    assert_eq!(parsed[1].query, "memes");
    assert_eq!(parsed[1].topic, "memes");
}
