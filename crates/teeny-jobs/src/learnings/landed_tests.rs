use teeny_store::posts::{Hot, MetricSample, OwnPost};

use super::*;

fn post(id: &str, kind: &str, day: &str, views: Option<i64>, engagements: i64) -> OwnPost {
    OwnPost {
        post_id: id.into(),
        kind: kind.into(),
        posted_at: format!("{day}T15:00:00.000Z"),
        text: Some(format!("the {kind} post {id}")),
        samples: views
            .map(|views| {
                vec![
                    // An early reading, then the latest: only the latest counts.
                    MetricSample {
                        at: format!("{day}T16:00:00.000Z"),
                        impressions: 1,
                        likes: 0,
                        replies: 0,
                        reposts: 0,
                        quotes: 0,
                    },
                    MetricSample {
                        at: format!("{day}T20:00:00.000Z"),
                        impressions: views,
                        likes: engagements,
                        replies: 0,
                        reposts: 0,
                        quotes: 0,
                    },
                ]
            })
            .unwrap_or_default(),
        hot: None,
    }
}

fn week() -> Vec<OwnPost> {
    let mut hot = post("a", "chart", "2026-10-04", Some(3_000), 90);
    hot.hot = Some(Hot {
        since: "2026-10-04T17:00:00.000Z".into(),
        hold_until: "2026-10-04T23:00:00.000Z".into(),
    });
    vec![
        hot,
        post("b", "chart", "2026-10-03", Some(900), 30),
        post("c", "morning", "2026-10-03", Some(1_000), 20),
        post("d", "lore", "2026-10-02", Some(200), 2),
        post("e", "moon", "2026-10-04", None, 0),
    ]
}

#[test]
fn reads_each_kind_and_the_posts_that_stood_out_from_the_latest_numbers() {
    let posts = week();
    let reading = assess(&posts);
    assert_eq!(reading.median_views, 950);
    assert_eq!(reading.counted, 4);
    assert_eq!(
        reading
            .by_kind
            .iter()
            .map(|kind| (kind.kind.as_str(), kind.posts, kind.views, kind.engagements))
            .collect::<Vec<_>>(),
        [
            ("chart", 2, 1_950, 60),
            ("morning", 1, 1_000, 20),
            ("lore", 1, 200, 2)
        ]
    );
    let ids = |posts: &[&OwnPost]| {
        posts
            .iter()
            .map(|post| post.post_id.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&reading.landed), ["a"]);
    assert_eq!(ids(&reading.flopped), ["d"]);
}

#[test]
fn renders_the_reading_for_the_prompt() {
    let text = render(&assess(&week()));
    assert_eq!(
        text,
        [
            "Over 4 posts with numbers; the median is 950 views.",
            "By kind, best first:",
            "- chart: 2 posts, 1950 views and 60 engagements on average",
            "- morning: 1 post, 1000 views and 20 engagements on average",
            "- lore: 1 post, 200 views and 2 engagements on average",
            "Landed (twice the median or more, or held hot):",
            "- [2026-10-04 chart] 3000 views, 90 engagements, HOT: the chart post a",
            "Flopped (half the median or less):",
            "- [2026-10-02 lore] 200 views, 2 engagements: the lore post d",
        ]
        .join("\n")
    );
}

#[test]
fn says_so_when_there_are_too_few_numbers_to_tell() {
    let few = vec![
        post("a", "chart", "2026-10-04", Some(500), 5),
        post("e", "moon", "2026-10-04", None, 0),
    ];
    assert_eq!(
        render(&assess(&few)),
        "(too few posts with numbers to tell: 1)"
    );
    // An even field with nothing far from the middle lands and flops nothing.
    let even = vec![
        post("a", "chart", "2026-10-04", Some(500), 5),
        post("b", "lore", "2026-10-03", Some(600), 5),
        post("c", "quote", "2026-10-02", Some(700), 5),
    ];
    let text = render(&assess(&even));
    assert!(
        text.contains("Landed (twice the median or more, or held hot):\n- (nothing stood out)"),
        "{text}"
    );
    assert!(
        text.ends_with("Flopped (half the median or less):\n- (nothing fell flat)"),
        "{text}"
    );
}
