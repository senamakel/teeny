//! The writers' room: teeny and its producer, alternating, for a fixed
//! number of exchanges. Each is a backrooms persona with its own agent
//! behind the runner (`AgentKey::Room`, scheduled, run once): teeny with
//! VOICE.md as its system prompt, the producer with its own brief. Neither
//! posts. The whole transcript is in every prompt, so a turn needs no
//! session. A turn that comes back empty is kept as `...`; two empty
//! exchanges in a row end the room.

use std::sync::{Arc, LazyLock};

use regex::Regex;

use super::stages::{
    PRODUCER_PROMPT, RoomContext, TEENY_PROMPT, Turn, Who, producer_turn_prompt, teeny_turn_prompt,
};
use crate::backrooms::ask::Ask;
use crate::runner::{AgentKey, AgentRun, AgentRunner, Origin};

/// The two personas' ids.
pub const TEENY: &str = "serial-teeny";
pub const PRODUCER: &str = "serial-producer";

async fn speak(runner: &Arc<dyn AgentRunner>, persona: &str, ask: Ask) -> anyhow::Result<String> {
    let outcome = runner
        .run(AgentRun {
            agent: AgentKey::Room {
                persona: persona.to_owned(),
                system_prompt: ask.system,
            },
            origin: Origin::Scheduled { once: true },
            prompt: ask.prompt,
            session: None,
            model: None,
            label: format!("narrative:{persona}"),
        })
        .await?;
    Ok(outcome.reply)
}

pub async fn run_writers_room(
    runner: &Arc<dyn AgentRunner>,
    context: &RoomContext,
    turns: u32,
) -> anyhow::Result<Vec<Turn>> {
    let mut transcript: Vec<Turn> = Vec::new();
    let mut silent = 0;
    for exchange in 0..turns {
        let last = exchange + 1 == turns;
        let teeny = clean(
            &speak(runner, TEENY, teeny_turn_prompt(context, &transcript)).await?,
            Who::Teeny,
        );
        transcript.push(Turn {
            who: Who::Teeny,
            text: if teeny.is_empty() {
                "...".into()
            } else {
                teeny.clone()
            },
        });
        let producer = clean(
            &speak(
                runner,
                PRODUCER,
                producer_turn_prompt(context, &transcript, last),
            )
            .await?,
            Who::Producer,
        );
        transcript.push(Turn {
            who: Who::Producer,
            text: if producer.is_empty() {
                "...".into()
            } else {
                producer.clone()
            },
        });
        silent = if teeny.is_empty() && producer.is_empty() {
            silent + 1
        } else {
            0
        };
        if silent >= 2 {
            break;
        }
    }
    Ok(transcript)
}

static TRAILING_TEENY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"\n?{}\s*$", regex::escape(TEENY_PROMPT))).expect("valid")
});
static TRAILING_PRODUCER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"\n?{}\s*$", regex::escape(PRODUCER_PROMPT))).expect("valid")
});

/// The turn, cut at the first line that belongs to the other speaker, its
/// own prompt marker stripped from the front.
pub fn clean(raw: &str, who: Who) -> String {
    let (own, other, trailing) = match who {
        Who::Teeny => (TEENY_PROMPT, PRODUCER_PROMPT, &*TRAILING_TEENY),
        Who::Producer => (PRODUCER_PROMPT, TEENY_PROMPT, &*TRAILING_PRODUCER),
    };
    let mut text = raw.trim().to_owned();
    if let Some(rest) = text.strip_prefix(own) {
        text = rest.trim().to_owned();
    }
    if text.starts_with(other) {
        let Some(resume) = text.find(&format!("\n{own}")) else {
            return String::new();
        };
        text = text[resume + 1 + own.len()..].trim().to_owned();
    }
    if let Some(cut) = text.find(&format!("\n{other}")) {
        text.truncate(cut);
    }
    trailing.replace(&text, "").trim().to_owned()
}

#[cfg(test)]
#[path = "room_tests.rs"]
mod tests;
