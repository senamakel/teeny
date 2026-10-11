//! What landed and what flopped, worked out from the post ledger's numbers
//! in code rather than left for the model to eyeball.
//!
//! The heat job samples every own post's public counters while it is young
//! (`own_posts.samples`); the latest sample is the post's numbers. Each kind
//! is averaged, and against the median of the posts that have numbers, a
//! post that drew twice the median or more (or was held hot) landed, and one
//! that drew half or less flopped. The model gets this beside the raw list,
//! so "What landed and what flopped" is written from a reading that is the
//! same every time for the same record.

use std::collections::BTreeMap;

use teeny_memory::text::clip;
use teeny_store::posts::{MetricSample, OwnPost};

/// Fewer posts with numbers than this, and there is no telling.
const MIN_POSTS: usize = 3;
/// How many posts each list shows.
const SHOWN: usize = 3;

/// One kind of post, averaged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KindReading {
    pub kind: String,
    pub posts: usize,
    /// Average views.
    pub views: i64,
    /// Average likes, replies, reposts and quotes, together.
    pub engagements: i64,
}

/// The reading of a stretch of posts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reading<'a> {
    /// Posts with at least one sample.
    pub counted: usize,
    pub median_views: i64,
    /// Best average views first.
    pub by_kind: Vec<KindReading>,
    /// Most views first.
    pub landed: Vec<&'a OwnPost>,
    /// Fewest views first.
    pub flopped: Vec<&'a OwnPost>,
}

/// A post's latest numbers.
fn latest(post: &OwnPost) -> Option<&MetricSample> {
    post.samples.iter().max_by(|a, b| a.at.cmp(&b.at))
}

fn views(post: &OwnPost) -> i64 {
    latest(post).map_or(0, |sample| sample.impressions)
}

fn engagements(post: &OwnPost) -> i64 {
    latest(post).map_or(0, |sample| {
        sample.likes + sample.replies + sample.reposts + sample.quotes
    })
}

/// Reads `posts`: by kind, and against the median.
pub fn assess(posts: &[OwnPost]) -> Reading<'_> {
    let counted: Vec<&OwnPost> = posts.iter().filter(|post| latest(post).is_some()).collect();
    let mut sorted: Vec<i64> = counted.iter().map(|post| views(post)).collect();
    sorted.sort_unstable();
    let median_views = match sorted.len() {
        0 => 0,
        n if n % 2 == 1 => sorted[n / 2],
        n => (sorted[n / 2 - 1] + sorted[n / 2]) / 2,
    };

    let mut kinds: BTreeMap<&str, (usize, i64, i64)> = BTreeMap::new();
    for post in &counted {
        let entry = kinds.entry(post.kind.as_str()).or_default();
        entry.0 += 1;
        entry.1 += views(post);
        entry.2 += engagements(post);
    }
    let mut by_kind: Vec<KindReading> = kinds
        .into_iter()
        .map(|(kind, (posts, views, engagements))| {
            let count = i64::try_from(posts).unwrap_or(i64::MAX);
            KindReading {
                kind: kind.to_owned(),
                posts,
                views: views / count,
                engagements: engagements / count,
            }
        })
        .collect();
    by_kind.sort_by(|a, b| b.views.cmp(&a.views).then_with(|| a.kind.cmp(&b.kind)));

    let mut landed: Vec<&OwnPost> = counted
        .iter()
        .copied()
        .filter(|post| post.hot.is_some() || (median_views > 0 && views(post) >= 2 * median_views))
        .collect();
    landed.sort_by_key(|post| std::cmp::Reverse(views(post)));
    landed.truncate(SHOWN);
    let mut flopped: Vec<&OwnPost> = counted
        .iter()
        .copied()
        .filter(|post| post.hot.is_none() && 2 * views(post) <= median_views)
        .collect();
    flopped.sort_by_key(|post| views(post));
    flopped.truncate(SHOWN);

    Reading {
        counted: counted.len(),
        median_views,
        by_kind,
        landed,
        flopped,
    }
}

fn line(post: &OwnPost) -> String {
    let day: String = post.posted_at.chars().take(10).collect();
    format!(
        "- [{day} {}] {} views, {} engagements{}: {}",
        post.kind,
        views(post),
        engagements(post),
        if post.hot.is_some() { ", HOT" } else { "" },
        clip(post.text.as_deref().unwrap_or_default(), 200)
    )
}

/// The reading as the prompt carries it.
pub fn render(reading: &Reading<'_>) -> String {
    if reading.counted < MIN_POSTS {
        return format!("(too few posts with numbers to tell: {})", reading.counted);
    }
    let mut lines = vec![
        format!(
            "Over {} posts with numbers; the median is {} views.",
            reading.counted, reading.median_views
        ),
        "By kind, best first:".to_owned(),
    ];
    lines.extend(reading.by_kind.iter().map(|kind| {
        format!(
            "- {}: {} post{}, {} views and {} engagements on average",
            kind.kind,
            kind.posts,
            if kind.posts == 1 { "" } else { "s" },
            kind.views,
            kind.engagements
        )
    }));
    lines.push("Landed (twice the median or more, or held hot):".to_owned());
    if reading.landed.is_empty() {
        lines.push("- (nothing stood out)".to_owned());
    }
    lines.extend(reading.landed.iter().map(|post| line(post)));
    lines.push("Flopped (half the median or less):".to_owned());
    if reading.flopped.is_empty() {
        lines.push("- (nothing fell flat)".to_owned());
    }
    lines.extend(reading.flopped.iter().map(|post| line(post)));
    lines.join("\n")
}

#[cfg(test)]
#[path = "landed_tests.rs"]
mod tests;
