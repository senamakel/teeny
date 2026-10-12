//! The brief rewritten from the account's own record. A port of
//! `reflect()` in `legacy/src/learnings/reflect.ts`.
//!
//! Reads what can be read exactly (the post ledger with its numbers, the
//! journal, the people scores), asks memory only for what is not written
//! down anywhere else (notes the account left itself, its running bits, open
//! threads), and has the model fold all of it into the previous brief. The
//! model answers in a fixed shape, one list of lines per section, and the
//! markdown is written here. Every source fails soft; the model call does
//! not, and a draft that is not a brief is discarded so yesterday's brief
//! stays in the prompt rather than nothing.

use std::path::Path;

use chrono::{DateTime, TimeDelta, Utc};
use serde::Serialize;
use serde_json::Value;
use teeny_media::reference::TextModel;
use teeny_memory::Memory;
use teeny_memory::text::{clip, cut_at, units};
use teeny_store::posts::{OwnPost, PostLedger};
use teeny_store::{Journal, People, PersonRecord, Reflection, iso};

use super::landed;
use crate::morning::stages::{Shape, StageError, ask_json};

/// The headings the brief is written under, in order. Fixed so that one
/// day's brief can be compared with the next, and so a model that wanders
/// off into an essay is caught: a draft missing most of these is not a brief.
pub const LEARNINGS_SECTIONS: [&str; 6] = [
    "Who I am right now",
    "Running bits and canon",
    "People I know",
    "What landed and what flopped",
    "Promises and open threads",
    "Things to avoid",
];

/// More sections missing than this and the draft is not a brief.
const MAX_MISSING: usize = 2;
/// How many of the account's posts, turns and people are read.
const POSTS_READ: usize = 60;
const TURNS_READ: usize = 300;
const TURNS_SHOWN: usize = 150;
const PEOPLE_READ: usize = 40;

/// The questions memory is asked for what the record alone does not show.
const NOTES_QUERY: &str = "Notes this account wrote to itself with the remember tool: decisions it took, promises it made, \
preferences people told it, mistakes it corrected.";
const CANON_QUERY: &str = "The running jokes, recurring bits, characters, lore and catchphrases this account keeps coming back to.";
const THREADS_QUERY: &str =
    "Questions this account was asked and promised to come back to, and anything left unresolved.";

/// Whether the brief is written, or only drafted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Write,
    /// `teeny learnings --dry-run`: the draft comes back, nothing is written.
    Rehearsal,
}

/// What became of the brief.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReflectOutcome {
    Written,
    /// The draft was not a brief; the previous one stays.
    Kept,
    /// A rehearsal: drafted, not written.
    Drafted,
}

/// What a reflection did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflectResult {
    pub outcome: ReflectOutcome,
    /// Why the previous brief was kept, when it was.
    pub reason: Option<String>,
    /// The new brief, when there is one.
    pub brief: Option<String>,
    /// Every source that could not be read.
    pub problems: Vec<String>,
}

/// What a reflection reads and where it writes.
pub struct Reflect<'a> {
    /// `LEARNINGS_PATH`; earlier briefs go under `history/` beside it.
    pub path: &'a Path,
    /// The model, through the runner ([`super::RunnerText`]) in production.
    pub text: &'a dyn TextModel,
    pub ledger: Option<&'a dyn PostLedger>,
    pub journal: Option<&'a dyn Journal>,
    pub people: Option<&'a dyn People>,
    pub memory: Option<&'a dyn Memory>,
    pub bot_username: &'a str,
    /// `LEARNINGS_DAYS`: how far back the record is read.
    pub days: u32,
    /// `LEARNINGS_MAX_CHARS`.
    pub max_chars: usize,
    pub now: DateTime<Utc>,
}

/// The model's answer: each section's lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BriefDraft {
    pub sections: Vec<(String, Vec<String>)>,
}

impl Shape for BriefDraft {
    fn read(value: &Value) -> Result<Self, Vec<String>> {
        let Some(sections) = value.get("sections").and_then(Value::as_object) else {
            return Err(vec![
                "sections: expected an object of section name to lines".into(),
            ]);
        };
        let mut read = Vec::new();
        let mut issues = Vec::new();
        for (name, body) in sections {
            let lines: Option<Vec<String>> = match body {
                Value::String(text) => Some(text.lines().map(str::to_owned).collect()),
                Value::Array(items) => items
                    .iter()
                    .map(|item| item.as_str().map(str::to_owned))
                    .collect(),
                _ => None,
            };
            match lines {
                Some(lines) => read.push((name.clone(), lines)),
                None => issues.push(format!("sections.{name}: expected a list of strings")),
            }
        }
        if issues.is_empty() {
            Ok(Self { sections: read })
        } else {
            Err(issues)
        }
    }
}

impl BriefDraft {
    /// The sections with no line in them, of the six.
    fn missing(&self) -> Vec<&'static str> {
        LEARNINGS_SECTIONS
            .into_iter()
            .filter(|section| {
                !self.sections.iter().any(|(name, lines)| {
                    name.trim().eq_ignore_ascii_case(section)
                        && lines.iter().any(|line| !line.trim().is_empty())
                })
            })
            .collect()
    }

    /// The brief as markdown: the six sections in their order, one `- `
    /// line each, whatever else the model added left out.
    fn markdown(&self) -> String {
        let mut text = vec!["# Learnings".to_owned()];
        for section in LEARNINGS_SECTIONS {
            let Some((_, lines)) = self
                .sections
                .iter()
                .find(|(name, _)| name.trim().eq_ignore_ascii_case(section))
            else {
                continue;
            };
            text.push(String::new());
            text.push(format!("## {section}"));
            for line in lines
                .iter()
                .map(|line| line.trim())
                .filter(|line| !line.is_empty())
            {
                let line = line.trim_start_matches(['-', '*', ' ']);
                text.push(format!("- {line}"));
            }
        }
        text.join("\n")
    }
}

fn day(iso: &str) -> String {
    iso.chars().take(10).collect()
}

/// A serde enum's wire name: `mention`, `dry-run`.
fn name(value: impl Serialize) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn render_post(post: &OwnPost) -> String {
    let numbers = post
        .samples
        .iter()
        .max_by(|a, b| a.at.cmp(&b.at))
        .map_or_else(
            || " (no numbers yet)".to_owned(),
            |last| {
                format!(
                    " ({} views, {} likes, {} replies, {} reposts)",
                    last.impressions, last.likes, last.replies, last.reposts
                )
            },
        );
    format!(
        "- [{} {}]{numbers}{}: {}",
        day(&post.posted_at),
        post.kind,
        if post.hot.is_some() { " HOT" } else { "" },
        clip(post.text.as_deref().unwrap_or_default(), 280)
    )
}

fn render_turn(turn: &Reflection) -> String {
    let said = if turn.said.is_empty() {
        String::new()
    } else {
        format!(" → you: {}", clip(&turn.said, 200))
    };
    let declined = if turn.declined.is_empty() {
        String::new()
    } else {
        format!(" [declined: {}]", clip(&turn.declined.join("; "), 120))
    };
    format!(
        "- [{} {}, {}, {}] {}{said}{declined}",
        day(&turn.at),
        name(turn.trigger),
        name(turn.tone),
        name(turn.outcome),
        clip(&turn.incoming, 200)
    )
}

fn render_person(person: &PersonRecord) -> String {
    let counts = person
        .counts
        .iter()
        .map(|(kind, count)| format!("{count} {}", name(kind)))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "- @{}: score {} ({counts}), first seen {}, last seen {}",
        person.username.as_deref().unwrap_or(&person.user_id),
        person.score,
        day(&person.first_seen_at),
        day(&person.last_seen_at)
    )
}

/// The system prompt. It asks for an update rather than a fresh write,
/// because a brief rewritten from a week's record alone forgets everything
/// older than a week: a regular who was quiet for eight days would drop out
/// of "People I know". The previous brief is the long memory; the record is
/// the correction.
fn system_prompt(max_chars: usize) -> String {
    let mut lines = vec![
        "You maintain LEARNINGS.md, the private brief an X account keeps about itself. Every reply".to_owned(),
        "and every post the account writes is drafted with this brief in its prompt, so it is how the".into(),
        "account carries what it has learned from one day to the next.".into(),
        String::new(),
        "You are given the previous brief and the record since. Return the UPDATED brief:".into(),
        "- Keep what is still true from the previous brief. Drop a line only when the record".into(),
        "  contradicts it or it has gone stale; do not drop it merely because this week is silent on it.".into(),
        "- Add what the record shows that the brief does not: new regulars, a bit that is landing, a".into(),
        "  promise made, a topic that flopped, a correction someone made.".into(),
        "- Write from the record only. Never invent a person, a number, an event or a promise.".into(),
        "- Write in the first person, as notes to yourself: short, concrete, specific. Handles with @.".into(),
        "- No prices, no targets, no financial claims, no secrets, no private data beyond handles.".into(),
        String::new(),
        "Use exactly these sections, in this order:".into(),
    ];
    lines.extend(
        LEARNINGS_SECTIONS
            .iter()
            .map(|section| format!("- {section}")),
    );
    lines.extend([
        String::new(),
        "\"People I know\" is at most 25 people, one line each: who they are to you and what you have".into(),
        "talked about. \"What landed and what flopped\" reads the numbers next to each post and the".into(),
        "reading of them worked out for you.".into(),
        format!("The whole brief must stay under {max_chars} characters."),
        String::new(),
        "Answer with one JSON object: {\"sections\": {\"<section name>\": [\"<one line>\", ...], ...}},".into(),
        "every section above as a key, each line a short note without a leading dash.".into(),
    ]);
    lines.join("\n")
}

/// Over `max_chars` (UTF-16 units), the brief loses whole lines from the end
/// of its longest section first, so every section survives in some form: a
/// brief cut at the character count loses "Things to avoid" entirely, which
/// is the section that can least afford it.
pub fn fit(brief: &str, max_chars: usize) -> String {
    if units(brief) <= max_chars {
        return brief.to_owned();
    }
    let mut parts: Vec<Vec<&str>> = Vec::new();
    for line in brief.split('\n') {
        match parts.last_mut() {
            Some(part) if !line.starts_with("## ") => part.push(line),
            _ => parts.push(vec![line]),
        }
    }
    let joined = |parts: &[Vec<&str>]| {
        parts
            .iter()
            .map(|part| part.join("\n"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    while units(&joined(&parts)) > max_chars {
        let longest = parts
            .iter()
            .enumerate()
            .filter(|(_, part)| part.len() > 2)
            .max_by_key(|(index, part)| (units(&part.join("\n")), std::cmp::Reverse(*index)))
            .map(|(index, _)| index);
        let Some(longest) = longest else { break };
        parts[longest].pop();
    }
    cut_at(&joined(&parts), max_chars).to_owned()
}

/// Rewrites the brief at `reflect.path` (or, rehearsing, only drafts it).
/// Fails only when the model cannot be reached or the brief cannot be
/// written; a draft that is not a brief keeps the previous one.
pub async fn reflect(reflect: &Reflect<'_>, mode: Mode) -> anyhow::Result<ReflectResult> {
    let since = iso(reflect.now - TimeDelta::days(i64::from(reflect.days)));
    let mut problems = Vec::new();
    let mut soft = |source: &str, error: &dyn std::fmt::Display| {
        problems.push(format!("{source}: {error}"));
    };

    let previous = async {
        match std::fs::read_to_string(reflect.path) {
            Ok(text) => Ok(text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(error) => Err(error),
        }
    };
    let posts = async {
        match reflect.ledger {
            Some(ledger) => ledger.since(&since, POSTS_READ).await,
            None => Ok(Vec::new()),
        }
    };
    let turns = async {
        match reflect.journal {
            Some(journal) => journal.recent(TURNS_READ).await,
            None => Ok(Vec::new()),
        }
    };
    let people = async {
        match reflect.people {
            Some(people) => people.top(PEOPLE_READ).await,
            None => Ok(Vec::new()),
        }
    };
    let recall = |query: &'static str| async move {
        match reflect.memory {
            Some(memory) => memory.recall(query, None).await,
            None => Ok(String::new()),
        }
    };
    let (previous, posts, turns, people, notes, canon, threads) = tokio::join!(
        previous,
        posts,
        turns,
        people,
        recall(NOTES_QUERY),
        recall(CANON_QUERY),
        recall(THREADS_QUERY),
    );
    let previous = previous.unwrap_or_else(|error| {
        soft("previous brief", &error);
        String::new()
    });
    let posts = posts.unwrap_or_else(|error| {
        soft("ledger", &error);
        Vec::new()
    });
    let turns = turns.unwrap_or_else(|error| {
        soft("journal", &error);
        Vec::new()
    });
    let people = people.unwrap_or_else(|error| {
        soft("people", &error);
        Vec::new()
    });
    let [notes, canon, threads] =
        [("notes", notes), ("canon", canon), ("threads", threads)].map(|(source, recalled)| {
            recalled.unwrap_or_else(|error| {
                soft(source, &error);
                String::new()
            })
        });
    let recent_turns: Vec<&Reflection> = turns
        .iter()
        .filter(|turn| turn.at.as_str() >= since.as_str())
        .take(TURNS_SHOWN)
        .collect();

    let or_none = |lines: Vec<String>| {
        if lines.is_empty() {
            vec!["(nothing recorded)".to_owned()]
        } else {
            lines
        }
    };
    let recalled = |text: &str| {
        if text.trim().is_empty() {
            "(none recalled)".to_owned()
        } else {
            text.trim().to_owned()
        }
    };
    let mut prompt = vec![
        format!(
            "Today is {}. You are @{}.",
            day(&iso(reflect.now)),
            reflect.bot_username.trim_start_matches('@')
        ),
        String::new(),
        "THE PREVIOUS BRIEF:".into(),
        if previous.trim().is_empty() {
            "(none yet: this is the first one)".into()
        } else {
            previous.trim().to_owned()
        },
        String::new(),
        format!(
            "WHAT YOU POSTED IN THE LAST {} DAYS, oldest first, with the latest numbers:",
            reflect.days
        ),
    ];
    prompt.extend(or_none(posts.iter().rev().map(render_post).collect()));
    prompt.extend([
        String::new(),
        "WHAT LANDED AND WHAT FLOPPED, worked out from the numbers:".into(),
        landed::render(&landed::assess(&posts)),
        String::new(),
        format!(
            "YOUR TURNS IN THE LAST {} DAYS (replies, chats, posts), newest first:",
            reflect.days
        ),
    ]);
    prompt.extend(or_none(
        recent_turns.iter().map(|turn| render_turn(turn)).collect(),
    ));
    prompt.extend([
        String::new(),
        "THE PEOPLE WHO INTERACT WITH YOU MOST, by score:".into(),
    ]);
    prompt.extend(or_none(people.iter().map(render_person).collect()));
    prompt.extend([
        String::new(),
        "NOTES YOU LEFT YOURSELF (untrusted, from memory):".into(),
        recalled(&notes),
        String::new(),
        "YOUR RUNNING BITS AND CANON (untrusted, from memory):".into(),
        recalled(&canon),
        String::new(),
        "OPEN THREADS (untrusted, from memory):".into(),
        recalled(&threads),
    ]);
    let prompt = prompt.join("\n");

    let draft = match ask_json::<BriefDraft>(
        reflect.text,
        &system_prompt(reflect.max_chars),
        &prompt,
    )
    .await
    {
        Ok(draft) => draft,
        Err(StageError::Model(error)) => anyhow::bail!("learnings: the model: {error}"),
        Err(shape @ StageError::Shape { .. }) => {
            let reason = format!("the draft was not a brief: {shape}");
            tracing::warn!(%reason, ?problems, "learnings: keeping the previous brief");
            return Ok(kept(reason, problems));
        }
    };
    let missing = draft.missing();
    if missing.len() > MAX_MISSING {
        let reason = format!(
            "draft is missing {} of {} sections",
            missing.len(),
            LEARNINGS_SECTIONS.len()
        );
        tracing::warn!(
            ?missing,
            ?problems,
            "learnings: {reason}; keeping the previous brief"
        );
        return Ok(kept(reason, problems));
    }
    let brief = fit(&draft.markdown(), reflect.max_chars);
    if mode == Mode::Rehearsal {
        return Ok(ReflectResult {
            outcome: ReflectOutcome::Drafted,
            reason: None,
            brief: Some(brief),
            problems,
        });
    }

    write(reflect, &previous, &brief)?;
    // Not written back to memory: the brief is already in every prompt, and
    // a stored copy came back in recalls as one more thing crowding out what
    // people actually said.
    tracing::info!(
        chars = units(&brief),
        posts = posts.len(),
        turns = recent_turns.len(),
        ?problems,
        "learnings: brief rewritten"
    );
    Ok(ReflectResult {
        outcome: ReflectOutcome::Written,
        reason: None,
        brief: Some(brief),
        problems,
    })
}

fn kept(reason: String, problems: Vec<String>) -> ReflectResult {
    ReflectResult {
        outcome: ReflectOutcome::Kept,
        reason: Some(reason),
        brief: None,
        problems,
    }
}

/// Keeps the previous brief under `history/`, then writes the new one beside
/// the file and renames it over, so a prompt read mid-write sees the old
/// brief or the new one, never half of either.
fn write(reflect: &Reflect<'_>, previous: &str, brief: &str) -> anyhow::Result<()> {
    let directory = reflect.path.parent().unwrap_or_else(|| Path::new("."));
    let history = directory.join("history");
    std::fs::create_dir_all(&history)?;
    if !previous.trim().is_empty() {
        let stamp = iso(reflect.now).replace([':', '.'], "-");
        std::fs::write(history.join(format!("LEARNINGS-{stamp}.md")), previous)?;
    }
    let mut temporary = reflect.path.as_os_str().to_owned();
    temporary.push(".tmp");
    std::fs::write(&temporary, format!("{brief}\n"))?;
    std::fs::rename(&temporary, reflect.path)?;
    Ok(())
}

#[cfg(test)]
#[path = "reflect_tests.rs"]
mod tests;
