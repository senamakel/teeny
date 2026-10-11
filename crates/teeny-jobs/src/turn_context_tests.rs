//! Port of `legacy/test/turn-context.test.ts`.

use std::sync::Mutex;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use teeny_memory::{Experience, MemoryError};
use teeny_store::posts::{InMemoryPostLedger, NewOwnPost};
use teeny_store::{Exchange, StoreError};

use super::*;

fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-10-05T12:00:00Z")
        .unwrap()
        .to_utc()
}

type Answer = Box<dyn Fn(&str) -> Result<String, String> + Send + Sync>;

/// Memory that answers each recall with `answer(query)` and records them.
struct Recalls {
    answer: Answer,
    queries: Mutex<Vec<(String, Option<Person>)>>,
}

impl Recalls {
    fn new(answer: impl Fn(&str) -> Result<String, String> + Send + Sync + 'static) -> Self {
        Self {
            answer: Box::new(answer),
            queries: Mutex::default(),
        }
    }

    fn queries(&self) -> Vec<(String, Option<Person>)> {
        self.queries.lock().unwrap().clone()
    }
}

#[async_trait]
impl Memory for Recalls {
    async fn capture(&self, _experience: Experience) -> teeny_memory::Result<()> {
        Ok(())
    }

    async fn recall(&self, query: &str, person: Option<&Person>) -> teeny_memory::Result<String> {
        self.queries
            .lock()
            .unwrap()
            .push((query.to_owned(), person.cloned()));
        (self.answer)(query).map_err(MemoryError::InvalidInput)
    }
}

/// An exchange history with fixed rows, or down.
struct Exchanges {
    rows: Result<Vec<Exchange>, String>,
    calls: Mutex<Vec<(String, usize, Option<String>)>>,
}

impl Exchanges {
    fn new(rows: Result<Vec<Exchange>, String>) -> Self {
        Self {
            rows,
            calls: Mutex::default(),
        }
    }
}

#[async_trait]
impl ExchangeHistory for Exchanges {
    async fn exchanges_with(
        &self,
        author_id: &str,
        limit: usize,
        exclude_post_id: Option<&str>,
    ) -> teeny_store::Result<Vec<Exchange>> {
        self.calls.lock().unwrap().push((
            author_id.to_owned(),
            limit,
            exclude_post_id.map(str::to_owned),
        ));
        self.rows
            .clone()
            .map_err(|error| StoreError::Io(std::io::Error::other(error)))
    }
}

async fn ledger_with(posts: &[(&str, &str, &str, &str)]) -> InMemoryPostLedger {
    let ledger = InMemoryPostLedger::default();
    for (post_id, kind, posted_at, text) in posts {
        ledger
            .record(&NewOwnPost {
                post_id: (*post_id).into(),
                kind: (*kind).into(),
                posted_at: (*posted_at).into(),
                text: Some((*text).into()),
            })
            .await
            .unwrap();
    }
    ledger
}

fn ana() -> Person {
    Person::x("7", Some("ana"))
}

#[tokio::test]
async fn reads_the_exchanges_and_the_own_posts_exactly_and_asks_memory_only_for_the_rest() {
    let memory = Recalls::new(|_| Ok("ABOUT @ana:\nlikes the lab lore".into()));
    let exchanges = Exchanges::new(Ok(vec![Exchange {
        at: "2026-10-04T09:00:00.000Z".into(),
        incoming: "is the serial back?".into(),
        reply: "tuesday, promise".into(),
        conversation_id: None,
    }]));
    let ledger = ledger_with(&[
        ("p1", "chart", "2026-10-05T07:00:00.000Z", "the chart today"),
        (
            "p0",
            "morning",
            "2026-09-20T07:00:00.000Z",
            "too old to show",
        ),
    ])
    .await;
    let context = TurnContext {
        memory: &memory,
        own_posts: Some(&ledger),
        exchanges: Some(&exchanges),
        limits: TurnLimits::default(),
        now: now(),
    };

    let person = ana();
    let turn = context
        .gather(TurnQuery {
            query: "q",
            person: Some(&person),
            author_id: Some("7"),
            post_id: Some("99"),
        })
        .await;

    assert_eq!(
        *exchanges.calls.lock().unwrap(),
        [("7".to_owned(), 6, Some("99".to_owned()))]
    );
    // One recall, not two: the own-activity question is answered by the ledger.
    assert_eq!(memory.queries().len(), 1);
    assert!(
        turn.text.contains("YOUR LAST 1 EXCHANGES WITH @ana"),
        "{}",
        turn.text
    );
    assert!(
        turn.text
            .contains("[2026-10-04 09:00 UTC] @ana: is the serial back?\n  you: tuesday, promise"),
        "{}",
        turn.text
    );
    assert!(turn.text.contains("likes the lab lore"));
    assert!(
        turn.text.contains(&format!(
            "{OWN_ACTIVITY_HEADING}\n[2026-10-05 07:00 UTC · chart] the chart today"
        )),
        "{}",
        turn.text
    );
    assert!(!turn.text.contains("too old to show"));
    assert!(turn.degraded.is_empty());
    assert!(
        turn.recalled
            .contains(&"1 earlier exchanges with @ana".to_owned())
    );
}

#[tokio::test]
async fn says_memory_was_unavailable_instead_of_passing_silence_as_nothing_known() {
    let memory = Recalls::new(|_| Err("request timed out after 15000ms".into()));
    let ledger = ledger_with(&[("p1", "moon", "2026-10-05T03:00:00.000Z", "moon chart")]).await;
    let context = TurnContext {
        memory: &memory,
        own_posts: Some(&ledger),
        exchanges: None,
        limits: TurnLimits::default(),
        now: now(),
    };
    let person = Person::x("7", None);
    let turn = context
        .gather(TurnQuery {
            query: "q",
            person: Some(&person),
            ..TurnQuery::default()
        })
        .await;

    assert!(
        turn.text.contains("MEMORY WAS UNAVAILABLE THIS TURN"),
        "{}",
        turn.text
    );
    // Said out loud, never the error itself.
    assert!(!turn.text.contains("15000"));
    // The other sources still arrive.
    assert!(turn.text.contains("moon chart"));
    assert_eq!(turn.degraded, ["memory"]);
    assert!(turn.recalled.contains(&"(memory unavailable)".to_owned()));
}

#[tokio::test]
async fn still_drafts_from_memory_when_the_exchange_history_cannot_be_read() {
    let memory = Recalls::new(|_| Ok("GENERAL CONTEXT:\nfacts".into()));
    let exchanges = Exchanges::new(Err("mongo down".into()));
    let ledger = ledger_with(&[]).await;
    let context = TurnContext {
        memory: &memory,
        own_posts: Some(&ledger),
        exchanges: Some(&exchanges),
        limits: TurnLimits::default(),
        now: now(),
    };
    let turn = context
        .gather(TurnQuery {
            query: "q",
            author_id: Some("7"),
            ..TurnQuery::default()
        })
        .await;
    assert_eq!(turn.text, "GENERAL CONTEXT:\nfacts");
    assert_eq!(turn.degraded, ["exchanges"]);
}

#[tokio::test]
async fn falls_back_to_asking_memory_for_its_own_posts_when_there_is_no_ledger() {
    let memory = Recalls::new(|query| {
        Ok(if query == OWN_ACTIVITY_QUERY {
            "posted a chart".into()
        } else {
            "context".into()
        })
    });
    let context = TurnContext {
        memory: &memory,
        own_posts: None,
        exchanges: None,
        limits: TurnLimits::default(),
        now: now(),
    };
    let turn = context
        .gather(TurnQuery {
            query: "q",
            ..TurnQuery::default()
        })
        .await;
    assert!(
        memory
            .queries()
            .contains(&(OWN_ACTIVITY_QUERY.to_owned(), None))
    );
    assert_eq!(
        turn.text,
        format!("context\n\n{OWN_ACTIVITY_HEADING}\nposted a chart")
    );
}

#[tokio::test]
async fn turns_a_source_off_at_zero() {
    let memory = Recalls::new(|_| Ok(String::new()));
    let exchanges = Exchanges::new(Ok(Vec::new()));
    let context = TurnContext {
        memory: &memory,
        own_posts: None,
        exchanges: Some(&exchanges),
        limits: TurnLimits {
            exchanges: 0,
            own_posts_limit: 0,
            ..TurnLimits::default()
        },
        now: now(),
    };
    context
        .gather(TurnQuery {
            query: "q",
            author_id: Some("7"),
            ..TurnQuery::default()
        })
        .await;
    assert!(exchanges.calls.lock().unwrap().is_empty());
    assert_eq!(memory.queries().len(), 1);
}

#[test]
fn reads_its_limits_from_the_configuration() {
    let config = teeny_core::Config::from_vars(
        teeny_agent::deps::fake_vars()
            .into_iter()
            .chain([("MEMORY_EXCHANGES", "3"), ("MEMORY_OWN_POSTS_HOURS", "24")]),
    )
    .unwrap();
    assert_eq!(
        TurnLimits::from_config(&config.memory),
        TurnLimits {
            exchanges: 3,
            own_posts_hours: 24,
            own_posts_limit: 8,
        }
    );
}
