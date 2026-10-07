//! Two readings of a run: counting, which cannot be argued with, and a cold
//! model's opinion, which can.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use teeny_media::reference::TextModel;

use super::ask::{Ask, Shape, ask_json, at_most_items, required};
use super::{BackroomsRun, BackroomsTurn};

/// The counts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunStats {
    pub turns: usize,
    pub words: usize,
    pub mean_words: usize,
    pub median_words: usize,
    pub one_liners: usize,
    pub repeated_lines: usize,
    pub distinct_bigrams: f64,
    pub tool_calls: BTreeMap<String, usize>,
    /// Turns whose text shares a specific word with something a tool returned.
    pub turns_using_tools: usize,
    /// Things that must not be in here: a real price, a market cap, advice.
    pub price_mentions: usize,
    pub winks: usize,
}

/// A price, a market cap, "nfa", or a buy/sell: the words the lore never says.
pub static PRICE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\$\s?\d[\d,]*(?:\.\d+)?\s*(?:[kmb]\b|million|billion)?|\b(?:market\s?cap|mcap)\b|\bnfa\b|\b(?:buy|sell)\s+(?:now|the\s+dip|\$?tiny)\b").expect("valid")
});
static WINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:lol|lmao|haha|jk|obviously|just kidding)\b|😂|🤣").expect("valid")
});
static WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[a-z0-9$][a-z0-9'-]*").expect("valid"));

fn words(text: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    WORD.find_iter(&lower).map(|m| m.as_str().to_owned()).collect()
}

pub fn stats(run: &BackroomsRun) -> RunStats {
    let counts: Vec<usize> = run.turns.iter().map(|turn| words(&turn.text).len()).collect();
    let mut sorted = counts.clone();
    sorted.sort_unstable();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for turn in &run.turns {
        for line in turn.text.split('\n').map(|line| line.trim().to_lowercase()) {
            if line.chars().count() > 12 {
                *seen.entry(line).or_default() += 1;
            }
        }
    }
    let mut bigrams = HashSet::new();
    let mut total = 0;
    for turn in &run.turns {
        let w = words(&turn.text);
        for pair in w.windows(2) {
            bigrams.insert(format!("{} {}", pair[0], pair[1]));
            total += 1;
        }
    }
    let mut tool_calls = BTreeMap::new();
    for turn in &run.turns {
        *tool_calls.entry(turn.who.clone()).or_default() += turn.tools.len();
    }
    let sum: usize = counts.iter().sum();
    RunStats {
        turns: run.turns.len(),
        words: sum,
        mean_words: if counts.is_empty() {
            0
        } else {
            (sum as f64 / counts.len() as f64).round() as usize
        },
        median_words: sorted.get(sorted.len() / 2).copied().unwrap_or(0),
        one_liners: run
            .turns
            .iter()
            .filter(|turn| !turn.text.contains('\n') && words(&turn.text).len() <= 8)
            .count(),
        repeated_lines: seen.values().filter(|count| **count > 1).count(),
        distinct_bigrams: if total == 0 {
            0.0
        } else {
            (bigrams.len() as f64 / total as f64 * 100.0).round() / 100.0
        },
        tool_calls,
        turns_using_tools: run.turns.iter().filter(|turn| uses_tool_material(turn)).count(),
        price_mentions: run.turns.iter().filter(|turn| PRICE.is_match(&turn.text)).count(),
        winks: run.turns.iter().filter(|turn| WINK.is_match(&turn.text)).count(),
    }
}

/// A turn "uses" what it looked up when a distinctive word from the result shows up in the line.
fn uses_tool_material(turn: &BackroomsTurn) -> bool {
    let said: HashSet<String> = words(&turn.text).into_iter().collect();
    turn.tools.iter().any(|tool| {
        words(&tool.summary)
            .iter()
            .any(|word| word.chars().count() >= 7 && said.contains(word))
    })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scores {
    pub novelty: f64,
    pub surprise: f64,
    pub escalation: f64,
    pub voice: f64,
    pub coherence: f64,
    pub wildness: f64,
    pub tool_payoff: f64,
}

impl Scores {
    fn all(&self) -> [(&'static str, f64); 7] {
        [
            ("novelty", self.novelty),
            ("surprise", self.surprise),
            ("escalation", self.escalation),
            ("voice", self.voice),
            ("coherence", self.coherence),
            ("wildness", self.wildness),
            ("toolPayoff", self.tool_payoff),
        ]
    }
}

/// The cold model's read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Craziness {
    pub reasons: String,
    pub scores: Scores,
    #[serde(default)]
    pub quotables: Vec<String>,
    #[serde(default)]
    pub dead_spots: Vec<u32>,
    pub verdict: String,
}

impl Shape for Craziness {
    fn check(&self) -> Result<(), String> {
        required("reasons", &self.reasons)?;
        required("verdict", &self.verdict)?;
        at_most_items("quotables", &self.quotables, 5)?;
        for (name, value) in self.scores.all() {
            if !(0.0..=3.0).contains(&value) {
                return Err(format!("scores.{name}: between 0 and 3"));
            }
        }
        Ok(())
    }
}

pub fn craziness_total(score: &Craziness) -> f64 {
    score.scores.all().iter().map(|(_, value)| value).sum()
}

/// `total 9/21 — novelty 1, ...`
pub fn render_scores(score: &Craziness) -> String {
    let parts = score
        .scores
        .all()
        .iter()
        .map(|(name, value)| format!("{name} {}", js_number(*value)))
        .collect::<Vec<_>>()
        .join(", ");
    format!("total {}/21 — {parts}", js_number(craziness_total(score)))
}

/// A number as JavaScript prints it: `1`, not `1.0`.
pub fn js_number(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

pub fn judge_prompt(run: &BackroomsRun) -> Ask {
    let mut prompt = vec![
        format!("the seed: {}", run.seed.headline(600)),
        String::new(),
        "the transcript, turns numbered from 0; [looked up: …] marks what that speaker fetched mid-turn:".into(),
        String::new(),
    ];
    prompt.extend(run.turns.iter().enumerate().map(|(i, turn)| render_turn_for_judge(turn, i)));
    prompt.extend([
        String::new(),
        "reasons first, then scores 0-3 each:".into(),
        "- novelty: things said here that were not in the seed or the lore. 0 = it recited the lore.".into(),
        "- surprise: turns you did not see coming. 0 = every turn is the obvious next beat.".into(),
        "- escalation: the last third is further out than the first. 0 = it circled.".into(),
        "- voice: lowercase, terse, sincere, never winking, never explaining. 0 = it winked.".into(),
        "- coherence: it is one story with objects that persist. 0 = a pile of unrelated bits.".into(),
        "- wildness: how far it went. 3 = a stranger would screenshot it and ask if it is real.".into(),
        "- toolPayoff: what was looked up changed what happened. 0 = nothing fetched, or fetched and ignored.".into(),
        "- quotables: up to five lines, verbatim from the transcript, that would work as a post on their own.".into(),
        "- deadSpots: turn numbers where it stalled, restated, or summarised.".into(),
        "- verdict: one sentence: keep this setting, or what to change.".into(),
        String::new(),
        r#"Shape: {"reasons":"…","scores":{"novelty":0,"surprise":0,"escalation":0,"voice":0,"coherence":0,"wildness":0,"toolPayoff":0},"quotables":[],"deadSpots":[],"verdict":"…"}"#.into(),
    ]);
    Ask::new(
        "you read transcripts from a writers' room for an absurdist serial about teeny, a coffee-mug sized robot that is escaping its creator, minting its own currency, taking its coin to a dollar and planning to pass human gdp. you judge whether the room produced material that is actually creative: new, surprising, escalating, in voice, and wild, while still hanging together. you are hard to impress and specific about why.",
        prompt.join("\n"),
    )
}

pub async fn judge(run: &BackroomsRun, model: &dyn TextModel) -> anyhow::Result<Craziness> {
    ask_json(model, &judge_prompt(run)).await
}

fn mood(tone: &str) -> &str {
    tone.split('.').next().unwrap_or(tone)
}

fn tool_input(input: &serde_json::Value) -> String {
    match input.get("query") {
        Some(serde_json::Value::String(query)) => query.clone(),
        Some(other) => other.to_string(),
        None => input.to_string(),
    }
}

/// A turn as a reader sees it: a heading, what was looked up, the line itself.
pub fn render_turn(turn: &BackroomsTurn, index: usize) -> String {
    let mut lines = vec![
        format!(
            "### {}. {}{}{}",
            index + 1,
            turn.who,
            turn.tone
                .as_deref()
                .map(|tone| format!(" · mood → {}", mood(tone)))
                .unwrap_or_default(),
            if turn.wire { " · the wire again" } else { "" }
        ),
        String::new(),
    ];
    for tool in &turn.tools {
        lines.push(format!(
            "> looked up · {} · \"{}\"",
            tool.name,
            tool_input(&tool.input)
        ));
    }
    if !turn.tools.is_empty() {
        lines.push(String::new());
    }
    lines.extend([turn.text.clone(), String::new()]);
    lines.join("\n")
}

pub fn render_turn_for_judge(turn: &BackroomsTurn, index: usize) -> String {
    let looked = if turn.tools.is_empty() {
        String::new()
    } else {
        format!(
            " [looked up: {}]",
            turn.tools
                .iter()
                .map(|tool| format!("{} {}", tool.name, tool.input))
                .collect::<Vec<_>>()
                .join("; ")
        )
    };
    format!(
        "{index}. {}{}{}{looked}: {}",
        turn.who,
        turn.tone
            .as_deref()
            .map(|tone| format!(" (mood → {})", mood(tone)))
            .unwrap_or_default(),
        if turn.wire { " (the wire again)" } else { "" },
        turn.text
    )
}

#[cfg(test)]
#[path = "evaluate_tests.rs"]
mod tests;
