//! Port of `legacy/test/learnings.test.ts`, `reflect` and `fit`.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use chrono::{DateTime, TimeDelta, Utc};
use serde_json::{Value, json};
use teeny_store::journal::{Outcome, Tone, Trigger};
use teeny_store::people::{Interaction, Observation};
use teeny_store::posts::{Hot, MetricSample, NewOwnPost, OwnPost};
use teeny_store::{
    InMemoryJournal, InMemoryPeople, InMemoryPostLedger, Journal, People, PersonRecord, PostLedger,
    Reflection, ReflectionInput, StoreError,
};

use super::*;
use crate::backrooms::testing::ScriptedText;
use crate::reply::test_support::ProbeMemory;

fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-10-05T05:00:00Z")
        .unwrap()
        .to_utc()
}

fn workspace(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "teeny-learnings-{name}-{}-{}",
        std::process::id(),
        now().timestamp_nanos_opt().unwrap_or_default()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

/// The model's structured answer: every section, with `lines` overriding some.
fn answer(lines: &[(&str, &[&str])]) -> String {
    let mut sections = serde_json::Map::new();
    for section in LEARNINGS_SECTIONS {
        let body = lines
            .iter()
            .find(|(name, _)| *name == section)
            .map_or_else(|| vec!["something true"], |(_, body)| body.to_vec());
        sections.insert(section.into(), json!(body));
    }
    json!({ "sections": Value::Object(sections) }).to_string()
}

/// The markdown that answer is written as.
fn brief(lines: &[(&str, &[&str])]) -> String {
    let mut text = vec!["# Learnings".to_owned()];
    for section in LEARNINGS_SECTIONS {
        let body = lines
            .iter()
            .find(|(name, _)| *name == section)
            .map_or_else(|| vec!["something true"], |(_, body)| body.to_vec());
        text.push(String::new());
        text.push(format!("## {section}"));
        text.extend(body.iter().map(|line| format!("- {line}")));
    }
    text.join("\n")
}

struct Record {
    ledger: InMemoryPostLedger,
    journal: InMemoryJournal,
    people: InMemoryPeople,
}

async fn record() -> Record {
    let ledger = InMemoryPostLedger::default();
    ledger
        .record(&NewOwnPost {
            post_id: "p1".into(),
            kind: "chart".into(),
            posted_at: "2026-10-04T15:00:00.000Z".into(),
            text: Some("the chart".into()),
        })
        .await
        .unwrap();
    ledger
        .sample(
            "p1",
            &MetricSample {
                at: "2026-10-04T18:00:00.000Z".into(),
                impressions: 900,
                likes: 40,
                replies: 6,
                reposts: 3,
                quotes: 0,
            },
        )
        .await
        .unwrap();
    let journal = InMemoryJournal::default();
    journal
        .record(ReflectionInput {
            id: None,
            at: Some("2026-10-04T16:00:00.000Z".into()),
            trigger: Trigger::Mention,
            incoming: "@teeny is the serial back?".into(),
            tone: Tone::Friendly,
            recalled: Vec::new(),
            tools: Vec::new(),
            thinking: String::new(),
            said: "tuesday, promise".into(),
            declined: Vec::new(),
            outcome: Outcome::Posted,
            outcome_note: None,
        })
        .await
        .unwrap();
    let people = InMemoryPeople::default();
    people
        .observe(&Observation {
            user_id: "7".into(),
            username: Some("ana".into()),
            kind: Interaction::Reply,
            at: Some("2026-10-04T16:00:00.000Z".into()),
        })
        .await
        .unwrap();
    Record {
        ledger,
        journal,
        people,
    }
}

fn reflect_on<'a>(path: &'a Path, text: &'a ScriptedText) -> Reflect<'a> {
    Reflect {
        path,
        text,
        ledger: None,
        journal: None,
        people: None,
        memory: None,
        bot_username: "teeny",
        days: 7,
        max_chars: 10_000,
        now: now(),
    }
}

#[tokio::test]
async fn folds_the_record_into_the_previous_brief_and_keeps_yesterdays_copy() {
    let root = workspace("folds");
    let path = root.join("memory").join("LEARNINGS.md");
    let first = ScriptedText::new(|_, _| answer(&[]));
    let written = reflect(
        &Reflect {
            now: now() - TimeDelta::days(1),
            ..reflect_on(&path, &first)
        },
        Mode::Write,
    )
    .await
    .unwrap();
    assert_eq!(written.outcome, ReflectOutcome::Written);

    let model = ScriptedText::new(|_, _| {
        answer(&[("People I know", &["@ana: asked when the serial is back"])])
    });
    let record = record().await;
    let memory = ProbeMemory {
        default: "a note".into(),
        ..ProbeMemory::default()
    };
    let result = reflect(
        &Reflect {
            ledger: Some(&record.ledger),
            journal: Some(&record.journal),
            people: Some(&record.people),
            memory: Some(&memory),
            ..reflect_on(&path, &model)
        },
        Mode::Write,
    )
    .await
    .unwrap();

    assert_eq!(result.outcome, ReflectOutcome::Written);
    assert!(result.problems.is_empty(), "{:?}", result.problems);
    let (system, prompt) = model.calls().remove(0);
    // An update, not a fresh write: the old brief is in the prompt and the
    // system prompt says to keep what is still true.
    assert!(system.contains("Keep what is still true from the previous brief"));
    assert!(
        prompt.contains("THE PREVIOUS BRIEF:\n# Learnings"),
        "{prompt}"
    );
    assert!(
        prompt
            .contains("[2026-10-04 chart] (900 views, 40 likes, 6 replies, 3 reposts): the chart"),
        "{prompt}"
    );
    assert!(prompt.contains("@teeny is the serial back? → you: tuesday, promise"));
    assert!(prompt.contains("@ana: score 3"), "{prompt}");
    // The numbers, read in code rather than left to the model.
    assert!(prompt.contains("WHAT LANDED AND WHAT FLOPPED, worked out from the numbers:"));
    assert!(prompt.contains("NOTES YOU LEFT YOURSELF (untrusted, from memory):\na note"));

    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("## People I know\n- @ana: asked when the serial is back"));
    assert_eq!(result.brief.as_deref(), Some(written.trim_end()));
    let history = std::fs::read_dir(root.join("memory").join("history"))
        .unwrap()
        .count();
    assert_eq!(history, 1);
    assert!(!path.with_extension("md.tmp").exists());
    // Not written back to memory: the brief is already in every prompt.
    assert!(memory.captured_kinds().is_empty());
}

#[tokio::test]
async fn keeps_the_previous_brief_when_the_draft_is_not_a_brief() {
    let root = workspace("kept");
    let path = root.join("LEARNINGS.md");
    std::fs::write(&path, brief(&[])).unwrap();

    let essay = ScriptedText::new(|_, _| "Sure! Here is an essay about my week.".into());
    let result = reflect(&reflect_on(&path, &essay), Mode::Write)
        .await
        .unwrap();
    assert_eq!(result.outcome, ReflectOutcome::Kept);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), brief(&[]));

    // Three of six sections is not a brief either.
    let partial = ScriptedText::new(|_, _| {
        json!({ "sections": {
            "Who I am right now": ["a robot"],
            "People I know": ["@ana"],
            "Things to avoid": ["prices"],
        } })
        .to_string()
    });
    let result = reflect(&reflect_on(&path, &partial), Mode::Write)
        .await
        .unwrap();
    assert_eq!(result.outcome, ReflectOutcome::Kept);
    assert_eq!(
        result.reason.as_deref(),
        Some("draft is missing 3 of 6 sections")
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), brief(&[]));
}

/// Every store down at once.
struct Down;

fn down() -> StoreError {
    StoreError::Io(std::io::Error::other("down"))
}

#[async_trait]
impl PostLedger for Down {
    async fn record(&self, _post: &NewOwnPost) -> teeny_store::Result<()> {
        Err(down())
    }
    async fn latest(&self) -> teeny_store::Result<Option<OwnPost>> {
        Err(down())
    }
    async fn since(&self, _since: &str, _limit: usize) -> teeny_store::Result<Vec<OwnPost>> {
        Err(down())
    }
    async fn get(&self, _post_id: &str) -> teeny_store::Result<Option<OwnPost>> {
        Err(down())
    }
    async fn sample(&self, _post_id: &str, _sample: &MetricSample) -> teeny_store::Result<()> {
        Err(down())
    }
    async fn set_hot(&self, _post_id: &str, _hot: &Hot) -> teeny_store::Result<()> {
        Err(down())
    }
    async fn hold(&self, _now: &str) -> teeny_store::Result<Option<OwnPost>> {
        Err(down())
    }
}

#[async_trait]
impl Journal for Down {
    async fn record(&self, _entry: ReflectionInput) -> teeny_store::Result<()> {
        Err(down())
    }
    async fn recent(&self, _limit: usize) -> teeny_store::Result<Vec<Reflection>> {
        Err(down())
    }
}

#[async_trait]
impl People for Down {
    async fn observe(&self, _event: &Observation) -> teeny_store::Result<()> {
        Err(down())
    }
    async fn scores(
        &self,
        _user_ids: &[String],
    ) -> teeny_store::Result<std::collections::HashMap<String, i64>> {
        Err(down())
    }
    async fn top(&self, _limit: usize) -> teeny_store::Result<Vec<PersonRecord>> {
        Err(down())
    }
}

#[tokio::test]
async fn writes_the_first_brief_from_nothing_and_survives_every_source_being_down() {
    let path = workspace("first").join("LEARNINGS.md");
    let model = ScriptedText::new(|_, _| format!("```json\n{}\n```", answer(&[])));
    let memory = ProbeMemory::broken();
    let result = reflect(
        &Reflect {
            ledger: Some(&Down),
            journal: Some(&Down),
            people: Some(&Down),
            memory: Some(&memory),
            ..reflect_on(&path, &model)
        },
        Mode::Write,
    )
    .await
    .unwrap();
    assert_eq!(result.outcome, ReflectOutcome::Written);
    assert!(result.problems.len() >= 4, "{:?}", result.problems);
    assert!(
        model.calls()[0]
            .1
            .contains("(none yet: this is the first one)")
    );
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .starts_with("# Learnings")
    );
}

#[tokio::test]
async fn a_rehearsal_drafts_the_brief_and_writes_nothing() {
    let path = workspace("rehearsal").join("LEARNINGS.md");
    let model = ScriptedText::new(|_, _| answer(&[]));
    let result = reflect(&reflect_on(&path, &model), Mode::Rehearsal)
        .await
        .unwrap();
    assert_eq!(result.outcome, ReflectOutcome::Drafted);
    assert_eq!(result.brief.as_deref(), Some(brief(&[]).as_str()));
    assert!(!path.exists());
}

#[tokio::test]
async fn a_model_that_cannot_be_reached_fails_the_run_and_keeps_the_brief() {
    let root = workspace("unreachable");
    let path = root.join("LEARNINGS.md");
    std::fs::write(&path, brief(&[])).unwrap();
    let dead = ScriptedText::queue(Vec::new());
    assert!(
        reflect(&reflect_on(&path, &dead), Mode::Write)
            .await
            .is_err()
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), brief(&[]));
}

#[test]
fn fit_trims_the_longest_section_first_so_every_heading_survives() {
    let people: Vec<String> = (0..200)
        .map(|index| format!("@person{index}: a regular"))
        .collect();
    let people: Vec<&str> = people.iter().map(String::as_str).collect();
    let long = brief(&[("People I know", &people)]);
    let fitted = fit(&long, 1_500);
    assert!(fitted.chars().count() <= 1_500);
    for section in LEARNINGS_SECTIONS {
        assert!(fitted.contains(&format!("## {section}")), "{section}");
    }
    assert_eq!(fit("short", 100), "short");
}
