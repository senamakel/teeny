use serde_json::json;

use super::*;

fn tokens(input: i64, output: i64) -> Tokens {
    Tokens {
        input,
        output,
        reasoning: 0,
        cached: 0,
        total: input + output,
    }
}

fn call(id: &str, lane: Lane, source: &str, tokens: Tokens) -> InferenceInput {
    InferenceInput {
        id: Some(id.into()),
        ..InferenceInput::new(lane, source, "m", tokens)
    }
}

/// The ledger's contract, run against every implementation.
async fn contract(ledger: &dyn InferenceLedger) {
    assert_eq!(ledger.totals().await.unwrap(), Totals::default());

    ledger
        .record(call("a", Lane::Mascot, "reply", tokens(100, 20)))
        .await
        .unwrap();
    ledger
        .record(call("b", Lane::Mascot, "chart", tokens(50, 5)))
        .await
        .unwrap();
    ledger
        .record(call("c", Lane::Brain, "scout", tokens(7, 3)))
        .await
        .unwrap();
    // A retried write counts once.
    ledger
        .record(call("a", Lane::Mascot, "reply", tokens(999, 999)))
        .await
        .unwrap();

    let totals = ledger.totals().await.unwrap();
    assert_eq!(totals.lanes.mascot.calls, 2);
    assert_eq!(totals.lanes.mascot.tokens.total, 175);
    assert_eq!(totals.lanes.brain.calls, 1);
    assert_eq!(totals.lanes.brain.tokens, tokens(7, 3));
    assert_eq!(totals.sources["mascot/reply"].calls, 1);
    assert_eq!(totals.sources["mascot/reply"].tokens.total, 120);
    assert_eq!(totals.sources["mascot/chart"].calls, 1);
    assert!(totals.lanes.mascot.first.is_some());
    assert!(totals.lanes.mascot.first <= totals.lanes.mascot.last);

    // Newest first, by lane, and before a time.
    let old = InferenceInput {
        at: Some("2000-01-01T01:00:00.000Z".into()),
        ..call("old", Lane::Brain, "scout", Tokens::default())
    };
    let new = InferenceInput {
        at: Some("2000-01-01T02:00:00.000Z".into()),
        ..call("new", Lane::Mascot, "reply", Tokens::default())
    };
    ledger.record(old).await.unwrap();
    ledger.record(new).await.unwrap();
    let ids = |rows: Vec<Inference>| rows.into_iter().map(|row| row.id).collect::<Vec<_>>();
    let all = ids(ledger.recent(10, &RecentFilter::default()).await.unwrap());
    assert_eq!(all.len(), 5);
    assert_eq!(&all[3..], ["new", "old"]);
    let brain = ids(ledger
        .recent(
            10,
            &RecentFilter {
                lane: Some(Lane::Brain),
                before: None,
            },
        )
        .await
        .unwrap());
    assert_eq!(brain, ["c", "old"]);
    let before = RecentFilter {
        lane: None,
        before: Some("2000-01-01T02:00:00.000Z".into()),
    };
    assert_eq!(ids(ledger.recent(10, &before).await.unwrap()), ["old"]);
    assert_eq!(
        ledger
            .recent(2, &RecentFilter::default())
            .await
            .unwrap()
            .len(),
        2
    );

    // The oldest and newest calls a lane has counted.
    let totals = ledger.totals().await.unwrap();
    assert_eq!(
        totals.lanes.brain.first.as_deref(),
        Some("2000-01-01T01:00:00.000Z")
    );
    assert_eq!(
        totals.lanes.mascot.first.as_deref(),
        Some("2000-01-01T02:00:00.000Z")
    );
}

#[tokio::test]
async fn sums_every_row_into_its_lane_and_its_source_and_never_counts_a_row_twice() {
    contract(&InMemoryLedger::default()).await;
}

#[test]
fn reads_an_openai_usage_block_details_and_all_and_tolerates_their_absence() {
    assert_eq!(
        tokens_from_usage(Some(&json!({
            "prompt_tokens": 120,
            "completion_tokens": 40,
            "total_tokens": 160,
            "prompt_tokens_details": { "cached_tokens": 100 },
            "completion_tokens_details": { "reasoning_tokens": 25 },
        }))),
        Tokens {
            input: 120,
            output: 40,
            reasoning: 25,
            cached: 100,
            total: 160
        }
    );
    assert_eq!(
        tokens_from_usage(Some(&json!({ "prompt_tokens": 3, "completion_tokens": 4 }))),
        Tokens {
            input: 3,
            output: 4,
            reasoning: 0,
            cached: 0,
            total: 7
        }
    );
    assert_eq!(tokens_from_usage(None), Tokens::default());
    // LangChain's own shape, as a fallback.
    assert_eq!(
        tokens_from_usage(Some(&json!({
            "input_tokens": 10, "output_tokens": 5, "total_tokens": 15,
            "output_token_details": { "reasoning": 2 }, "input_token_details": { "cache_read": 4 },
        }))),
        Tokens {
            input: 10,
            output: 5,
            reasoning: 2,
            cached: 4,
            total: 15
        }
    );
    // A present but unusable value is a zero, not a fall-through.
    assert_eq!(
        tokens_from_usage(Some(&json!({ "prompt_tokens": "9", "input_tokens": 5 }))).input,
        0
    );
}

#[test]
fn blanks_handles_in_what_it_shows() {
    let row = redact_inference(&Inference {
        input: Some(vec![LedgerMessage {
            role: Role::User,
            tool: None,
            text: "from @fren: gm".into(),
        }]),
        reasoning: Some("they are @fren".into()),
        output: Some("gm @fren".into()),
        ..InferenceInput::new(Lane::Mascot, "reply", "m", Tokens::default()).complete()
    });
    assert_eq!(row.input.unwrap()[0].text, "from @…: gm");
    assert_eq!(row.reasoning.as_deref(), Some("they are @…"));
    assert_eq!(row.output.as_deref(), Some("gm @…"));
}

#[test]
fn cuts_a_message_and_digests_a_request() {
    assert_eq!(cut("  a\r\nb  ", MAX_MESSAGE_CHARS), "a\nb");
    let long = cut(&"x".repeat(2_500), MAX_MESSAGE_CHARS);
    assert_eq!(long.chars().count(), 2_000);
    assert!(long.ends_with('…'));
    assert_eq!(
        sha256(&["abc"]),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(sha256(&["a", "bc"]), sha256(&["abc"]));
}

#[test]
fn an_inference_is_spelled_as_the_typescript_service_stored_it() {
    let row = Inference {
        id: "i".into(),
        at: "2026-09-14T01:00:00.000Z".into(),
        kind: Some("inference".into()),
        input: Some(vec![LedgerMessage {
            role: Role::Tool,
            tool: Some("recall_memory".into()),
            text: "x".into(),
        }]),
        response_id: Some("chatcmpl-1".into()),
        prompt_digest: Some("p".into()),
        duration_ms: Some(12),
        ..InferenceInput::new(Lane::Brain, "scout", "m", tokens(1, 2)).complete()
    };
    assert_eq!(
        serde_json::to_value(&row).unwrap(),
        json!({
            "id": "i", "at": "2026-09-14T01:00:00.000Z", "lane": "brain", "source": "scout",
            "kind": "inference", "model": "m",
            "input": [{ "role": "tool", "tool": "recall_memory", "text": "x" }],
            "tokens": { "input": 1, "output": 2, "reasoning": 0, "cached": 0, "total": 3 },
            "responseId": "chatcmpl-1", "promptDigest": "p", "durationMs": 12,
        })
    );
}

struct Broken;

#[async_trait::async_trait]
impl InferenceLedger for Broken {
    async fn record(&self, _entry: InferenceInput) -> crate::Result<()> {
        Err(std::io::Error::other("mongo down").into())
    }
    async fn recent(&self, _limit: usize, _filter: &RecentFilter) -> crate::Result<Vec<Inference>> {
        Ok(vec![])
    }
    async fn totals(&self) -> crate::Result<Totals> {
        Ok(Totals::default())
    }
}

#[tokio::test]
async fn a_broken_ledger_costs_a_log_line_never_the_caller() {
    record_quietly(
        Some(&Broken),
        InferenceInput::new(Lane::Mascot, "reply", "m", Tokens::default()),
    )
    .await;
    record_quietly(
        None,
        InferenceInput::new(Lane::Mascot, "reply", "m", Tokens::default()),
    )
    .await;
}

#[tokio::test]
#[ignore = "needs a Mongo at TEENY_TEST_MONGO_URI"]
async fn the_mongo_ledger_keeps_the_contract() {
    use mongodb::bson::{Document, doc};

    let Some(db) = crate::mongo::testing::database().await else {
        return;
    };
    let name = crate::mongo::testing::collection("inferences");
    contract(&MongoLedger::with_collection(&db, &name)).await;

    // Totals the Node driver wrote as Doubles, past two billion, still read.
    let sums = db.collection::<Document>(&format!("{name}_totals"));
    sums.update_one(
        doc! { "_id": "totals" },
        doc! { "$set": { "lanes.brain.tokens.total": 3.0e9_f64 } },
    )
    .await
    .unwrap();
    let totals = MongoLedger::with_collection(&db, &name)
        .totals()
        .await
        .unwrap();
    assert_eq!(totals.lanes.brain.tokens.total, 3_000_000_000);

    db.collection::<Document>(&name).drop().await.unwrap();
    sums.drop().await.unwrap();
}
