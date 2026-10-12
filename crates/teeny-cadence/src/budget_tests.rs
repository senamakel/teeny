//! Port of "the reply budget" in `legacy/test/cadence.test.ts`, and of
//! `mentionsDeliberately` in `legacy/test/person.test.ts`, which the budget
//! uses to tell a call from a carried-along handle.

use std::sync::Arc;

use chrono::{DateTime, Utc};

use teeny_store::posts::{Hot, InMemoryPostLedger, PostLedger};

use super::*;
use crate::clock::{ManualClock, iso};
use crate::test_support::{HOUR, config, own, post, t0};

const BOT: &str = "2094854245468000256";

/// Answers `count_replied` from a list of past replies.
struct Replied(Vec<(ReplyLane, Option<&'static str>, DateTime<Utc>)>);

#[async_trait]
impl RepliedCounter for Replied {
    async fn count_replied(&self, filter: &RepliedFilter) -> teeny_store::Result<u64> {
        let since = filter
            .since
            .as_deref()
            .map(|since| DateTime::parse_from_rfc3339(since).unwrap().to_utc());
        Ok(self
            .0
            .iter()
            .filter(|(lane, own_post_id, at)| {
                *lane == filter.lane
                    && filter
                        .own_post_id
                        .as_deref()
                        .is_none_or(|id| *own_post_id == Some(id))
                    && since.is_none_or(|since| *at >= since)
            })
            .count() as u64)
    }
}

fn spent(
    lane: ReplyLane,
    own_post_id: Option<&'static str>,
    n: usize,
) -> Vec<(ReplyLane, Option<&'static str>, DateTime<Utc>)> {
    vec![(lane, own_post_id, t0() + HOUR); n]
}

fn subject(
    replied: Vec<(ReplyLane, Option<&'static str>, DateTime<Utc>)>,
) -> (Arc<InMemoryPostLedger>, ReplyBudget) {
    let ledger = Arc::new(InMemoryPostLedger::default());
    let rules = BudgetRules {
        auto_per_post: 10,
        smart_per_post: 10,
        gap: HOUR * 8,
        bot_username: Some("bot".into()),
        thread_window: TimeDelta::days(7),
    };
    let clock = Arc::new(ManualClock::new(t0() + HOUR * 2));
    (
        ledger.clone(),
        ReplyBudget::new(ledger, Arc::new(Replied(replied)), rules, clock),
    )
}

fn mention(text: &str) -> Mention {
    Mention {
        text: text.into(),
        conversation_id: None,
        in_reply_to_user_id: None,
    }
}

fn under(text: &str, conversation: &str, in_reply_to: &str) -> Mention {
    Mention {
        text: text.into(),
        conversation_id: Some(conversation.into()),
        in_reply_to_user_id: Some(in_reply_to.into()),
    }
}

fn assessment(
    lane: ReplyLane,
    own_post_id: Option<&str>,
    allowed: bool,
    used: u64,
    cap: Option<u64>,
) -> Assessment {
    Assessment {
        lane,
        own_post_id: own_post_id.map(str::to_owned),
        allowed,
        used,
        cap,
    }
}

#[test]
fn finds_the_own_post_a_reply_sits_under_by_conversation_or_by_falling_back_to_the_latest() {
    let own = [
        post("2", "chart", t0() + HOUR, vec![]),
        post("1", "morning", t0(), vec![]),
    ];
    let found = |m: &Mention| own_post_of(m, BOT, &own).map(|p| p.post_id.clone());
    assert_eq!(found(&under("x", "1", BOT)).as_deref(), Some("1"));
    let replying = Mention {
        in_reply_to_user_id: Some(BOT.into()),
        ..mention("x")
    };
    assert_eq!(found(&replying).as_deref(), Some("2"));
    assert_eq!(found(&mention("@bot hi")), None);
    assert_eq!(found(&under("x", "theirs", "someone")), None);
    // A reply to the account in somebody else's thread is not under an own
    // post, and must not be charged to the latest one.
    assert_eq!(found(&under("x", "theirs", BOT)), None);
}

#[tokio::test]
async fn caps_the_auto_lane_at_ten_under_each_post() {
    let (ledger, budget) = subject(spent(ReplyLane::Auto, Some("1"), 10));
    ledger.record(&own("1", "morning", t0())).await.unwrap();
    ledger
        .record(&own("2", "chart", t0() + HOUR))
        .await
        .unwrap();
    assert_eq!(
        budget.assess(&under("nice", "1", BOT), BOT).await.unwrap(),
        assessment(ReplyLane::Auto, Some("1"), false, 10, Some(10))
    );
    assert_eq!(
        budget.assess(&under("nice", "2", BOT), BOT).await.unwrap(),
        assessment(ReplyLane::Auto, Some("2"), true, 0, Some(10))
    );
}

#[tokio::test]
async fn gives_a_hot_post_the_held_slots_allowance_on_top_of_its_own() {
    let (ledger, budget) = subject(vec![]);
    ledger.record(&own("1", "morning", t0())).await.unwrap();
    let hot = Hot {
        since: iso(t0() + HOUR),
        hold_until: iso(t0() + HOUR * 17),
    };
    ledger.set_hot("1", &hot).await.unwrap();
    assert_eq!(
        held_slots(&ledger.get("1").await.unwrap().unwrap(), HOUR * 8),
        2
    );
    let assessed = budget.assess(&under("wow", "1", BOT), BOT).await.unwrap();
    assert_eq!(
        (assessed.lane, assessed.cap),
        (ReplyLane::Auto, Some(10 + 2 * 20))
    );
}

#[tokio::test]
async fn caps_the_smart_lane_per_own_post_published_in_the_last_day_one_posts_worth_at_least() {
    let (ledger, budget) = subject(spent(ReplyLane::Smart, None, 10));
    // A quote, or a reply in somebody else's thread where the handle was
    // carried along rather than typed: the reply guy's lane.
    let quote = mention("lol look at this");
    assert_eq!(
        budget.assess(&quote, BOT).await.unwrap(),
        assessment(ReplyLane::Smart, None, false, 10, Some(10))
    );
    ledger.record(&own("1", "morning", t0())).await.unwrap();
    ledger
        .record(&own("2", "chart", t0() + HOUR))
        .await
        .unwrap();
    ledger
        .record(&own("0", "moon", t0() - HOUR * 30))
        .await
        .unwrap();
    assert_eq!(
        budget.assess(&quote, BOT).await.unwrap(),
        assessment(ReplyLane::Smart, None, true, 10, Some(20))
    );
}

#[tokio::test]
async fn always_answers_a_post_that_typed_the_handle_on_purpose_whatever_the_days_budget_says() {
    let mut replied = spent(ReplyLane::Smart, None, 10);
    replied.extend(spent(ReplyLane::Called, None, 40));
    let (_, budget) = subject(replied);
    assert_eq!(
        budget
            .assess(&mention("hey @bot what do you make of this"), BOT)
            .await
            .unwrap(),
        assessment(ReplyLane::Called, None, true, 40, None)
    );
    // Deep in somebody else's thread, still called in by name.
    let deep = budget
        .assess(
            &under(
                "@someone @other come on @bot settle this",
                "theirs",
                "someone",
            ),
            BOT,
        )
        .await;
    assert_eq!(
        (deep.as_ref().unwrap().lane, deep.unwrap().allowed),
        (ReplyLane::Called, true)
    );
    // The handle carried along by X at the front of a reply is not a call.
    let carried = budget
        .assess(
            &under(
                "@someone @bot rhyming is a bold choice",
                "theirs",
                "someone",
            ),
            BOT,
        )
        .await;
    assert_eq!(
        (carried.as_ref().unwrap().lane, carried.unwrap().allowed),
        (ReplyLane::Smart, false)
    );
}

#[tokio::test]
async fn keeps_a_comment_under_the_accounts_own_post_in_the_auto_lane_even_when_it_names_the_account()
 {
    let (ledger, budget) = subject(spent(ReplyLane::Auto, Some("1"), 10));
    ledger.record(&own("1", "morning", t0())).await.unwrap();
    assert_eq!(
        budget
            .assess(&under("@bot hey @bot are you awake", "1", BOT), BOT)
            .await
            .unwrap(),
        assessment(ReplyLane::Auto, Some("1"), false, 10, Some(10))
    );
}

#[test]
fn reads_the_allowances_from_the_configuration() {
    let config = config();
    let rules = BudgetRules::from_config(&config);
    assert_eq!((rules.auto_per_post, rules.smart_per_post), (10, 10));
    assert_eq!(rules.gap, HOUR * 7);
    assert_eq!(rules.bot_username.as_deref(), Some("teenydroid"));
    assert_eq!(rules.thread_window, TimeDelta::days(7));
}

mod mentions_deliberately {
    use super::super::mentions_deliberately;

    #[test]
    fn sees_a_handle_typed_in_the_body() {
        assert!(mentions_deliberately(
            "@senamakel Oh hi fren @TinyMascot",
            "TinyMascot"
        ));
    }

    #[test]
    fn ignores_the_leading_run_x_prefixes_onto_a_reply() {
        assert!(!mentions_deliberately(
            "@iamgaurangdesai @TinyMascot wen ser",
            "TinyMascot"
        ));
    }

    #[test]
    fn sees_a_plain_top_level_mention_as_addressed_not_called() {
        assert!(!mentions_deliberately("@TinyMascot gm", "TinyMascot"));
    }

    #[test]
    fn does_not_match_a_longer_handle_that_starts_the_same_way() {
        assert!(!mentions_deliberately("hi @TinyMascotFan", "TinyMascot"));
    }

    #[test]
    fn does_not_read_a_retweet_as_a_call() {
        assert!(!mentions_deliberately(
            "RT @someone: hey @TinyMascot look",
            "@TinyMascot"
        ));
    }
}

mod queue_counter {
    use serde::{Deserialize, Serialize};
    use teeny_store::mentions::{InMemoryMentionQueue, MentionQueue, QueuedPost, Reply};

    use super::*;

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct Post {
        id: String,
    }

    impl QueuedPost for Post {
        fn post_id(&self) -> &str {
            &self.id
        }
    }

    #[tokio::test]
    async fn counts_replies_through_any_teeny_store_mention_queue() {
        let queue = Arc::new(InMemoryMentionQueue::<Post>::default());
        queue.cache(&[Post { id: "10".into() }]).await.unwrap();
        let reply = Reply {
            id: Some("99".into()),
            text: "gm".into(),
            lane: Some(ReplyLane::Auto),
            own_post_id: Some("1".into()),
        };
        queue.mark_replied("10", &reply).await.unwrap();
        let counter = QueueCounter::new(queue);
        let filter = RepliedFilter {
            lane: ReplyLane::Auto,
            own_post_id: Some("1".into()),
            since: None,
        };
        assert_eq!(counter.count_replied(&filter).await.unwrap(), 1);
    }
}
