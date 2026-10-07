use std::sync::Arc;

use chrono::TimeDelta;
use teeny_x::model::XPostMetrics;

use super::*;
use crate::morning::test_support::{FakeReads, at};

pub(crate) const OWN: &str = "2AF7CqwieUjUPALL7icuZtL3X7wENdjUjGBMmfV2pump";

pub(crate) fn now() -> DateTime<Utc> {
    at("2026-09-21T23:00:00Z")
}

pub(crate) fn metrics(id: &str, likes: u64, replies: u64, reposts: u64, quotes: u64) -> XPostMetrics {
    XPostMetrics {
        id: id.into(),
        likes,
        replies,
        reposts,
        quotes,
        impressions: None,
        bookmarks: None,
    }
}

pub(crate) fn post(id: &str, text: &str) -> XPost {
    XPost {
        author_username: Some(format!("user{id}")),
        created_at: Some(teeny_store::iso(now() - TimeDelta::hours(3))),
        metrics: Some(metrics(id, 3, 1, 0, 0)),
        ..XPost::new(id, text)
    }
}

pub(crate) fn by(author: &str, mut post: XPost) -> XPost {
    post.author_username = Some(author.into());
    post
}

pub(crate) fn rules() -> CandidateRules {
    CandidateRules {
        bot_username: "teenydroid".into(),
        trusted_handles: vec!["TinyHumansAISol".into(), "senamakel".into()],
        own_token_addresses: vec![OWN.into()],
        own_symbol: "TINY".into(),
        quoted: Vec::new(),
        max_age: TimeDelta::hours(48),
        author_cooldown: TimeDelta::days(7),
        screen: None,
        now: now(),
    }
}

fn quoted(author: &str, days_ago: i64) -> Vec<QuotedPost> {
    vec![QuotedPost {
        post_id: "1".into(),
        author_username: Some(author.into()),
        quoted_at: teeny_store::iso(now() - TimeDelta::days(days_ago)),
        own_post_id: None,
    }]
}

#[test]
fn keeps_a_bull_posting_the_ticker_and_one_pasting_our_contract() {
    assert_eq!(reason_to_skip(&post("1", "one year from now $TINY is the AI meme everybody wishes they held"), &rules()), None);
    let pasted = format!("still adding. solana:{OWN} is the only bag that matters");
    assert_eq!(reason_to_skip(&post("2", &pasted), &rules()), None);
}

#[test]
fn never_quotes_itself_the_same_post_twice_or_an_author_inside_the_cooldown() {
    assert_eq!(
        reason_to_skip(&by("TeenyDroid", post("1", "$TINY is the one")), &rules()).as_deref(),
        Some("our own post")
    );
    let recent = CandidateRules { quoted: quoted("user1", 2), ..rules() };
    assert_eq!(reason_to_skip(&post("1", "$TINY is the one and only"), &recent).as_deref(), Some("already quoted"));
    let skip = reason_to_skip(&by("user1", post("9", "$TINY is the one and only")), &recent).unwrap();
    assert!(skip.contains("was quoted on 2026-09-19"), "{skip}");
    let old = CandidateRules { quoted: quoted("user1", 9), ..rules() };
    assert_eq!(reason_to_skip(&by("user1", post("9", "$TINY is the one and only")), &old), None);
}

#[test]
fn drops_what_is_old_what_sells_something_else_and_what_says_nothing() {
    let mut old = post("1", "$TINY forever and ever");
    old.created_at = Some(teeny_store::iso(now() - TimeDelta::days(3)));
    assert_eq!(reason_to_skip(&old, &rules()).as_deref(), Some("older than the window"));
    assert_eq!(
        reason_to_skip(&post("2", "$TINY is cool but $JEANPHIL is the real play"), &rules()).as_deref(),
        Some("names another token ($JEANPHIL)")
    );
    assert_eq!(
        reason_to_skip(&post("3", "buy $TINY and 7GCihgDB8fe6KNjn2MYtkzZcRjQy3t9GHdC8uHYmW2hr now"), &rules()).as_deref(),
        Some("carries a contract address that is not ours")
    );
    assert_eq!(
        reason_to_skip(&post("4", "$TINY giveaway, dm me for the whitelist"), &rules()).as_deref(),
        Some("reads as promotion")
    );
    let bare = format!("@TinyHumansAISol solana:{OWN} 🫰");
    assert_eq!(reason_to_skip(&post("5", &bare), &rules()).as_deref(), Some("says nothing beyond the ticker"));
    assert_eq!(reason_to_skip(&post("6", "Attention $TINY family, less than 100 votes are needed"), &rules()), None);
}

#[test]
fn reads_the_content_filter_when_given_one() {
    let screened = CandidateRules {
        screen: Some(Arc::new(|text: &str| text != "$TINY is the one and only")),
        ..rules()
    };
    assert_eq!(
        reason_to_skip(&post("1", "$TINY is the one and only"), &screened).as_deref(),
        Some("the content filter")
    );
}

#[test]
fn scores_a_conversation_the_way_the_ranker_counts_it() {
    let mut busy = post("1", "x");
    busy.metrics = Some(metrics("1", 4, 2, 1, 1));
    assert_eq!(conversation_score(&busy), 14);
    assert_eq!(conversation_score(&XPost::new("1", "x")), 0);
}

#[tokio::test]
async fn runs_both_searches_dedupes_and_sorts_the_team_first_then_the_busiest() {
    let mut listing = post("2", "$TINY family, vote for the listing");
    listing.metrics = Some(metrics("2", 40, 3, 2, 0));
    let mut team = by("TinyHumansAISol", post("4", "@teenydroid you are the best bot on the timeline"));
    team.in_reply_to_user_id = Some("99".into());
    let reads = FakeReads::answering([
        Ok(vec![
            post("1", "$TINY is going to be huge, the narrative is right"),
            listing,
            by("teenydroid", post("3", "$TINY forever")),
        ]),
        Ok(vec![post("2", "$TINY family, vote for the listing"), team]),
    ]);
    let found = find_candidates(&reads, "$TINY OR \"abc\"", 50, 10, &rules()).await;
    let searched = reads.searched();
    assert_eq!(searched[0].0, "($TINY OR \"abc\") -is:retweet -from:teenydroid");
    assert_eq!(searched[1].0, "(@teenydroid OR to:teenydroid) -from:teenydroid -is:retweet");
    assert_eq!(searched[0].1.max_results, Some(50));
    assert_eq!(searched[0].1.sort_order, Some(SortOrder::Relevancy));
    assert_eq!(found.searched, 5);
    let ids: Vec<_> = found.candidates.iter().map(|c| c.post.id.to_string()).collect();
    assert_eq!(ids, ["4", "2", "1"]);
    assert!(found.candidates[0].trusted);
    assert_eq!(found.candidates[0].source, CandidateSource::Mention);
    assert_eq!(
        found.skipped,
        [Skipped { id: "3".into(), author: Some("teenydroid".into()), reason: "our own post".into() }]
    );
}

#[tokio::test]
async fn lets_one_author_have_two_in_the_list_not_seven() {
    let mut posts: Vec<XPost> = (1..=4)
        .map(|n| {
            let mut loud = by("loud", post(&n.to_string(), &format!("$TINY reason number {n} to hold")));
            loud.metrics = Some(metrics(&n.to_string(), 10 - n, 0, 0, 0));
            loud
        })
        .collect();
    posts.push(by("quiet", post("9", "$TINY is the one for me")));
    let reads = FakeReads::answering([Ok(posts), Ok(Vec::new())]);
    let found = find_candidates(&reads, "$TINY", 10, 10, &rules()).await;
    let ids: Vec<_> = found.candidates.iter().map(|c| c.post.id.to_string()).collect();
    assert_eq!(ids, ["1", "2", "9"]);
    let skipped: Vec<_> = found.skipped.iter().map(|s| s.id.clone()).collect();
    assert_eq!(skipped, ["3", "4"]);
}

#[tokio::test]
async fn survives_one_search_failing() {
    let reads = FakeReads::answering([
        Err(teeny_x::XError::Missing("429".into())),
        Ok(vec![post("1", "@teenydroid love what you are doing")]),
    ]);
    let found = find_candidates(&reads, "$TINY", 10, 10, &rules()).await;
    assert_eq!(found.candidates.len(), 1);
}
