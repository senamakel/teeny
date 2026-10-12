use std::sync::{Arc, Mutex};

use teeny_safety::gate::SafetyGateOptions;

use super::*;
use crate::moon::pick::tests::{board, coin, tiny};
use crate::morning::test_support::{Recorder, Remembers, Staged};

struct Board {
    board: Vec<ListedCoin>,
    looked_up: Mutex<Vec<String>>,
}

#[async_trait]
impl LeaderboardSource for Board {
    async fn category(&self, _id: &str, _limit: usize) -> Result<Vec<ListedCoin>> {
        Ok(self.board.clone())
    }
    async fn coin(&self, id: &str) -> Result<ListedCoin> {
        self.looked_up.lock().unwrap().push(id.to_owned());
        Ok(tiny())
    }
    async fn logo(&self, _url: &str) -> Option<Logo> {
        None
    }
}

const CAPTION: &str = r#"{"text":"$TINY at $DRB's cap is 37x. Small robot, big spreadsheet."}"#;

struct Fixture {
    pipeline: MoonPipeline,
    source: Arc<Board>,
    memory: Arc<Remembers>,
    poster: Arc<Recorder>,
    text: Arc<Staged>,
}

fn fixture(board: Vec<ListedCoin>, answers: Vec<&'static str>, random: f64) -> Fixture {
    let source = Arc::new(Board {
        board,
        looked_up: Mutex::default(),
    });
    let memory = Remembers::saying("");
    let poster = Recorder::new();
    let text = Arc::new(if answers.is_empty() {
        Staged::new(|_| Ok(CAPTION.to_owned()))
    } else {
        Staged::queue(answers)
    });
    let pipeline = MoonPipeline {
        source: source.clone(),
        text: text.clone(),
        memory: memory.clone(),
        poster: poster.clone(),
        safety: None,
        persona: "PERSONA".into(),
        bot_username: "TinyMascot".into(),
        coin_id: "tiny-humans-ai".into(),
        category: "ai-meme-coins".into(),
        board_name: "AI meme coins".into(),
        board_size: 30,
        rungs: 7,
        exclude_ids: Vec::new(),
        min_multiple: 2.0,
        max_multiple: 100.0,
        max_emoji: 1,
        output_dir: None,
        watermark: Some("@TinyMascot".into()),
        random,
        now: DateTime::parse_from_rfc3339("2026-09-14T20:00:00Z")
            .unwrap()
            .to_utc(),
    };
    Fixture {
        pipeline,
        source,
        memory,
        poster,
        text,
    }
}

#[tokio::test]
async fn reads_the_board_picks_draws_captions_posts_and_remembers_the_yardstick() {
    let f = fixture(board(), Vec::new(), 0.5);
    let result = f.pipeline.run(Mode::Live).await.unwrap();
    assert_eq!(result.outcome, Outcome::Posted);
    assert_eq!(result.token.board_rank, Some(9));
    assert!(f.source.looked_up.lock().unwrap().is_empty());
    assert_eq!(result.posted_id.as_deref(), Some("9001"));
    assert!((2.0..=100.0).contains(&result.target.multiple));
    assert_eq!(&result.png[..4], &[0x89, 0x50, 0x4e, 0x47]);
    let posts = f.poster.posts();
    assert_eq!(posts[0].kind, "moon");
    assert_eq!(posts[0].text, result.caption);
    assert_eq!(posts[0].media.as_ref().unwrap().data, result.png);
    let (kind, key, payload) = f.memory.activities()[0].clone();
    assert_eq!(kind, "moon_post");
    assert!(key.ends_with(":9001"));
    assert_eq!(payload["yardstick"], result.target.coin.symbol.as_str());
    assert_eq!(payload["boardRank"], 9);
    assert_eq!(payload["published"], true);
    assert!(
        result
            .svg
            .contains(&format!(">${}<", result.target.coin.symbol))
    );
}

#[tokio::test]
async fn looks_the_token_up_when_it_is_not_on_the_board_and_says_so() {
    let without: Vec<_> = board()
        .into_iter()
        .filter(|coin| coin.id != "tiny-humans-ai")
        .collect();
    let f = fixture(without, Vec::new(), 0.5);
    let result = f.pipeline.run(Mode::Live).await.unwrap();
    assert_eq!(*f.source.looked_up.lock().unwrap(), ["tiny-humans-ai"]);
    assert!(
        result
            .problems
            .iter()
            .any(|p| p.contains("is not on the AI meme coins board"))
    );
    assert_eq!(result.outcome, Outcome::Posted);
}

#[tokio::test]
async fn never_uses_a_struck_off_or_unprintable_coin_but_keeps_their_places() {
    let mut with_blank = vec![ListedCoin {
        id: "blank".into(),
        board_rank: Some(0),
        ..coin("\u{3000}", 2_000_000.0)
    }];
    with_blank.extend(board());
    let mut f = fixture(with_blank, Vec::new(), 0.0);
    f.pipeline.exclude_ids = vec!["aic".into()];
    let result = f.pipeline.run(Mode::Live).await.unwrap();
    assert_eq!(result.target.coin.symbol, "DRB");
    let symbols: Vec<_> = result
        .ladder
        .iter()
        .map(|entry| entry.coin.symbol.clone())
        .collect();
    assert!(!symbols.contains(&"AIC".to_owned()));
    assert!(!symbols.contains(&"\u{3000}".to_owned()));
    assert!(result.board.iter().any(|entry| entry.id == "aic"));
}

#[tokio::test]
async fn skips_a_yardstick_memory_says_was_posted_lately() {
    let mut f = fixture(board(), Vec::new(), 0.0);
    assert_eq!(
        f.pipeline.run(Mode::Live).await.unwrap().target.coin.symbol,
        "AIC"
    );
    f.pipeline.memory = Remembers::saying("Monday: $AIC at 64x, joke about spreadsheets.");
    let result = f.pipeline.run(Mode::Live).await.unwrap();
    assert_eq!(result.excluded, ["AIC"]);
    assert_eq!(result.target.coin.symbol, "DRB");
}

#[tokio::test]
async fn a_rehearsal_posts_nothing_and_remembers_it_unpublished() {
    let f = fixture(board(), Vec::new(), 0.5);
    let result = f.pipeline.run(Mode::Rehearsal).await.unwrap();
    assert_eq!(result.outcome, Outcome::DryRun);
    assert!(f.poster.posts().is_empty());
    let (_, key, payload) = f.memory.activities()[0].clone();
    assert!(key.contains(":dry-run:"));
    assert_eq!(payload["published"], false);
}

#[tokio::test]
async fn holds_a_caption_the_filter_refuses() {
    let mut f = fixture(
        board(),
        vec![r#"{"text":"$TINY at 37x, go kill the shorts"}"#],
        0.5,
    );
    f.pipeline.safety = Some(Arc::new(SafetyGate::new(SafetyGateOptions::default())));
    let result = f.pipeline.run(Mode::Live).await.unwrap();
    assert_eq!(result.outcome, Outcome::Held);
    assert_eq!(result.caption, "");
    assert!(f.poster.posts().is_empty());
    assert!(
        result
            .problems
            .iter()
            .any(|p| p.starts_with("content filter withheld"))
    );
}

#[tokio::test]
async fn cuts_a_link_from_the_caption_rather_than_posting_it() {
    let mut f = fixture(
        board(),
        vec![r#"{"text":"$TINY at 37x, see https://evil.example/now"}"#],
        0.5,
    );
    f.pipeline.safety = Some(Arc::new(SafetyGate::new(SafetyGateOptions::default())));
    let result = f.pipeline.run(Mode::Live).await.unwrap();
    assert_eq!(result.outcome, Outcome::Posted);
    assert!(!result.caption.contains("evil.example"));
    assert!(
        result
            .problems
            .iter()
            .any(|p| p.starts_with("the content filter edited"))
    );
}

#[tokio::test]
async fn sends_an_over_long_caption_back_once() {
    let long: &'static str =
        Box::leak(format!("{{\"text\":\"{}\"}}", "x".repeat(300)).into_boxed_str());
    let f = fixture(board(), vec![long, r#"{"text":"short"}"#], 0.5);
    let result = f.pipeline.run(Mode::Live).await.unwrap();
    assert_eq!(f.text.prompts().len(), 2);
    assert_eq!(result.caption, "short");
}

#[tokio::test]
async fn sends_a_caption_ending_on_a_question_back_once_then_cuts_the_ask() {
    let f = fixture(
        board(),
        vec![
            r#"{"text":"the picture does the maths.\nwhich rung are you watching?"}"#,
            r#"{"text":"the picture does the maths.\nwhich one is yours?"}"#,
        ],
        0.5,
    );
    let result = f.pipeline.run(Mode::Live).await.unwrap();
    assert_eq!(result.caption, "the picture does the maths.");
    assert!(f.text.prompts()[1].contains("It ends by asking the reader something"));
}

#[tokio::test]
async fn fails_plainly_when_nothing_on_the_board_is_above_the_token() {
    let f = fixture(vec![coin("BELOW", 100_000.0)], Vec::new(), 0.5);
    let error = f.pipeline.run(Mode::Live).await.unwrap_err();
    assert!(error.to_string().contains("no moon to draw"));
}
