//! The room itself: agents take turns, each spoken to with the previous
//! line, each answering in its own voice. The seed opens it. Every few
//! turns the mood shifts for whoever is speaking, so the register moves.
//! Halfway through, the wire is put back in front of whoever is speaking.
//! Two silent turns in a row end it; a room where nobody talks is over.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use async_trait::async_trait;
use regex::Regex;

use super::screen::quote_untrusted;
use super::text::{flatten, head};
use super::tokens::Random;
use super::tones::drift_tone;
use super::{AgentContext, AgentSpec, BackroomsTurn, Seed, ToolUse};
use crate::runner::{AgentKey, AgentRun, AgentRunner, Origin};

/// What a voice said, and what it looked up saying it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Spoken {
    pub text: String,
    pub tools: Vec<ToolUse>,
}

/// One voice in the room. It is spoken to, may look things up, and
/// answers. What it looked up stays in its own history.
#[async_trait]
pub trait RoomAgent: Send + Sync {
    async fn speak(&self, message: &str, system_note: Option<&str>) -> anyhow::Result<Spoken>;
}

/// A persona as its own agent behind the runner: `AgentKey::Room` with the
/// rendered context as its system prompt, a scheduled turn that runs once,
/// and a private session so its tool results and its own earlier lines are
/// what it sees, and nobody else's are. The `Room` belt never posts.
pub struct RunnerAgent {
    pub runner: Arc<dyn AgentRunner>,
    pub persona: String,
    pub system_prompt: String,
    pub session: String,
    pub label: String,
}

impl RunnerAgent {
    pub fn new(runner: Arc<dyn AgentRunner>, run_id: &str, context: &AgentContext) -> Self {
        Self {
            runner,
            persona: context.agent.id.clone(),
            system_prompt: render_system(context),
            session: format!("backrooms:{run_id}:{}", context.agent.id),
            label: format!("backrooms:{}", context.agent.id),
        }
    }
}

#[async_trait]
impl RoomAgent for RunnerAgent {
    async fn speak(&self, message: &str, system_note: Option<&str>) -> anyhow::Result<Spoken> {
        let prompt = match system_note {
            Some(note) => format!("<SYSTEM MESSAGE> {note}\n\n{message}"),
            None => message.to_owned(),
        };
        let outcome = self
            .runner
            .run(AgentRun {
                agent: AgentKey::Room {
                    persona: self.persona.clone(),
                    system_prompt: self.system_prompt.clone(),
                },
                origin: Origin::Scheduled { once: true },
                prompt,
                session: Some(self.session.clone()),
                model: None,
                label: self.label.clone(),
            })
            .await?;
        Ok(Spoken {
            text: outcome.reply,
            tools: outcome
                .tool_calls
                .into_iter()
                .map(|call| ToolUse {
                    name: call.name,
                    input: call.arguments,
                    summary: call.result,
                })
                .collect(),
        })
    }
}

/// The standing instructions, then every context block under its label.
pub fn render_system(context: &AgentContext) -> String {
    let mut lines = vec![context.agent.system.clone(), String::new()];
    for block in &context.blocks {
        lines.extend([
            format!("## {}", block.label),
            String::new(),
            block.text.clone(),
            String::new(),
        ]);
    }
    lines.push(format!(
        "you speak as {}. answer with your line only: no marker, no name, nothing",
        context.agent.marker
    ));
    lines.push("for anyone but the room. never write the other side's lines.".into());
    lines.join("\n")
}

/// One seat at the table.
pub struct Seat<'a> {
    pub spec: &'a AgentSpec,
    pub agent: &'a dyn RoomAgent,
    pub context: &'a AgentContext,
}

/// Runs the room for `turns` turns.
pub async fn run_backrooms(
    seats: &[Seat<'_>],
    seed: &Seed,
    turns: u32,
    random: &mut dyn Random,
    drift_every: u32,
) -> anyhow::Result<Vec<BackroomsTurn>> {
    if seats.is_empty() {
        return Ok(Vec::new());
    }
    let drift_every = drift_every.max(1);
    let mut transcript: Vec<BackroomsTurn> = Vec::new();
    let mut moods: HashMap<String, String> = seats
        .iter()
        .map(|seat| (seat.spec.id.clone(), seat.context.tone.clone()))
        .collect();
    let mut silent = 0;
    for index in 0..turns {
        let speaker = &seats[index as usize % seats.len()];
        let message = match transcript.last() {
            Some(previous) => format!("{} {}", marker_of(seats, &previous.who), previous.text),
            None => opener(seed, seats.len() > 1),
        };
        let mut tone = None;
        if index > 0 && index % drift_every == 0 {
            let current = moods.get(&speaker.spec.id).cloned().unwrap_or_default();
            let next = drift_tone(random, &current).to_owned();
            if next != current {
                moods.insert(speaker.spec.id.clone(), next.clone());
                tone = Some(next);
            }
        }
        let wire = index > 0 && index == turns / 2;
        let notes: Vec<String> = tone
            .iter()
            .map(|tone| format!("the mood shifts: {tone}"))
            .chain(wire.then(|| wire_again(seed)))
            .collect();
        let note = (!notes.is_empty()).then(|| notes.join("\n"));
        let others: Vec<&str> = seats
            .iter()
            .filter(|other| other.spec.id != speaker.spec.id)
            .map(|other| other.spec.marker.as_str())
            .collect();
        let mut spoken = speaker.agent.speak(&message, note.as_deref()).await?;
        let mut text = clean(&spoken.text, &speaker.spec.marker, &others);
        if looks_like_salad(&text) {
            // A hot model sometimes comes apart mid-turn. One more go, shorter; then the turn is lost.
            spoken = speaker
                .agent
                .speak(
                    "that came out garbled, none of it was words. say it again, in three lines at most.",
                    Some("the last line was noise. keep it plain and short."),
                )
                .await?;
            text = clean(&spoken.text, &speaker.spec.marker, &others);
            if looks_like_salad(&text) {
                text.clear();
            }
        }
        silent = if text.is_empty() { silent + 1 } else { 0 };
        transcript.push(BackroomsTurn {
            who: speaker.spec.id.clone(),
            text: if text.is_empty() { "...".into() } else { text },
            tone,
            wire,
            tools: spoken.tools,
        });
        if silent >= 2 {
            break;
        }
    }
    Ok(transcript)
}

static WORDS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[A-Za-z][A-Za-z'-]*").expect("valid"));

const COMMON: &str = "the a an and or but of to in on at for with from by is are was were be been it its this that these those i you we they he she me my your our their him her them not no yes so if then than as up down out off over under into about what when where who how why one two do does did done have has had can could will would should may might just very there here now still again never always all some any more most one thing things something nothing door room lab kettle agent agents teeny swarm";

/// Word salad: a hot model drifting into a stream of rare, capitalised,
/// unrelated words. A real turn, however strange, is mostly small common
/// words in lowercase.
pub fn looks_like_salad(text: &str) -> bool {
    let words: Vec<&str> = WORDS.find_iter(text).map(|m| m.as_str()).collect();
    if words.len() < 40 {
        return false;
    }
    let total = words.len() as f64;
    let common = words
        .iter()
        .filter(|word| COMMON.split(' ').any(|c| c == word.to_lowercase()))
        .count() as f64
        / total;
    let capitalised = words
        .iter()
        .filter(|word| word.starts_with(|c: char| c.is_ascii_uppercase()))
        .count() as f64
        / total;
    let long = words
        .iter()
        .filter(|word| word.chars().count() >= 9)
        .count() as f64
        / total;
    common < 0.22 || capitalised > 0.25 || long > 0.3
}

fn marker_of(seats: &[Seat<'_>], id: &str) -> String {
    seats
        .iter()
        .find(|seat| seat.spec.id == id)
        .map_or_else(|| format!("{id}>"), |seat| seat.spec.marker.clone())
}

/// The first line of the room: the seed, quoted, as what came through the
/// wire. It is untrusted text and reads as such.
fn opener(seed: &Seed, others: bool) -> String {
    let body = match &seed.title {
        Some(title) => format!("{title}\n{}", seed.text),
        None => seed.text.clone(),
    };
    [
        format!(
            "<SYSTEM MESSAGE> the room is open. {}",
            if others {
                "the others have no memory of anything you know unless you say it."
            } else {
                "nobody else is here yet."
            }
        ),
        "this came through the wire just now:".into(),
        String::new(),
        quote_untrusted(&body),
        String::new(),
        "go.".into(),
    ]
    .join("\n")
}

/// The seed, put back in front of a speaker mid-room, as the thing tonight is about.
pub fn wire_again(seed: &Seed) -> String {
    let what = head(
        &flatten(&quote_untrusted(
            seed.title.as_deref().unwrap_or(&seed.text),
        )),
        200,
    );
    [
        format!("the wire again, in case the room forgot: {what}"),
        "that is what tonight is about. whatever the lab is doing right now, it is doing because of that:".into(),
        "bring it back in, as a thing that is happening in here, in one specific object or line.".into(),
    ]
    .join("\n")
}

/// A model asked to continue a transcript often continues it: its own
/// marker on the front, the other side's next line after. The turn is the
/// text between its own marker and the first line that belongs to anyone
/// else.
pub fn clean(raw: &str, own: &str, others: &[&str]) -> String {
    let mut text = raw.trim().to_owned();
    if let Some(rest) = text.strip_prefix(own) {
        text = rest.trim().to_owned();
    }
    for other in others {
        if text.starts_with(other) {
            let Some(resume) = text.find(&format!("\n{own}")) else {
                return String::new();
            };
            text = text[resume + 1 + own.len()..].trim().to_owned();
        }
    }
    let cut = others
        .iter()
        .filter_map(|other| text.find(&format!("\n{other}")))
        .min()
        .unwrap_or(text.len());
    text.truncate(cut);
    let trailing = Regex::new(&format!(r"\n?{}\s*$", regex::escape(own))).expect("valid");
    trailing.replace(&text, "").trim().to_owned()
}

#[cfg(test)]
#[path = "room_tests.rs"]
mod tests;
