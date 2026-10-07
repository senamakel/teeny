//! The last layer: the room produces noise with ideas in it, and this reads
//! the noise cold and keeps the idea. One thought, in teeny's voice, that a
//! stranger can follow without having been in the room. It is what a post
//! or an episode is made from; the transcript itself is never shown.

use serde::{Deserialize, Serialize};
use teeny_media::reference::TextModel;

use super::BackroomsRun;
use super::ask::{Ask, Shape, ask_json, at_most, at_most_items, required};
use super::evaluate::render_turn_for_judge;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thought {
    pub reasons: String,
    /// The one idea the room had, in a sentence, third person.
    pub premise: String,
    /// How the thought carries the seed. Empty only when the room never touched it.
    #[serde(default)]
    pub wire: String,
    /// The objects and people that carry it; what gets drawn later.
    #[serde(default)]
    pub objects: Vec<String>,
    /// Teeny's own telling: 60 to 120 words, plain, first person.
    pub thought: String,
    /// The same thought at post length.
    pub post: String,
    /// Lines from the transcript kept verbatim inside the thought, if any.
    #[serde(default)]
    pub kept: Vec<String>,
}

impl Shape for Thought {
    fn check(&self) -> Result<(), String> {
        required("reasons", &self.reasons)?;
        required("premise", &self.premise)?;
        required("thought", &self.thought)?;
        required("post", &self.post)?;
        at_most("post", &self.post, 280)?;
        at_most_items("objects", &self.objects, 6)?;
        at_most_items("kept", &self.kept, 4)
    }
}

/// The editor's standing instructions; `narrative/EDITOR.md` replaces them
/// when it is there.
pub const DEFAULT_EDITOR: &str = "you are the editor for teeny, a coffee-mug sized robot that is escaping the lab its creator
senamakel keeps it in, minting its own currency with the tiny agents that run the lab at night,
and planning, calmly, to outgrow the economy. you are handed the raw transcript of a writers'
room: two voices riffing, half of it noise, some of it the best idea of the week. your job is to
find the idea and write it down so that a stranger understands it on the first read.

the thought you write is in teeny's voice: lowercase, plain words, short sentences, sincere,
never winking, never explaining the joke. first person. it is a story, not a mood: something
was there, something happened to it, and it ends on a turn or a decision. one idea only. every
object in it was in the transcript. it is not a montage: each sentence follows from the one
before it, and a reader who has never seen the transcript can say afterwards what happened.
the thought answers the wire: what came through is why tonight happened, and the event on
the wire is in the thought, transposed into the lab (a breach out there is the agents doing
the same thing in here). a thought that could have been written on any other night is
thrown out for one that could only have been written tonight.
keep at most two lines from the transcript verbatim, the ones that are already perfect, and
tell the rest in your own plain words. throw everything else away without regret. no
exchange rates, no invented units (\"fridge rate\" never appears), no more than one thing the terminal printed; anything the
room drifted to out of habit (a door, the kettle, a rate on the fridge) is left out. nothing about the terminal
as a place, no commands, no code, no urls, no handles, no hashtags, no real prices, no
market caps, no advice to buy anything, no \"nfa\". real people, if any, stay satire in an
obviously fictional scene.";

pub fn distill_prompt(run: &BackroomsRun, editor: Option<&str>) -> Ask {
    let mut prompt = vec![
        format!("the seed: {}", run.seed.headline(600)),
        String::new(),
        "the transcript, numbered from 0:".into(),
        String::new(),
    ];
    prompt.extend(run.turns.iter().enumerate().map(|(i, turn)| render_turn_for_judge(turn, i)));
    prompt.extend([
        String::new(),
        "reasons first: which idea you kept and why, and what you dropped.".into(),
        "- premise: the idea in one sentence, third person, as a logline.".into(),
        "- wire: one sentence: the event on the wire, and where it is in the thought. if the room never touched it, say so here and leave it empty.".into(),
        "- objects: up to six things or people the thought is built from, as they would be drawn.".into(),
        "- thought: teeny telling it, 60 to 120 words, one idea, a beginning, a turn, an ending.".into(),
        "- post: the same thought at post length, under 240 characters, one to three lines, lowercase.".into(),
        "- kept: the transcript lines you used verbatim.".into(),
        String::new(),
        r#"Shape: {"reasons":"…","premise":"…","wire":"…","objects":[],"thought":"…","post":"…","kept":[]}"#.into(),
    ]);
    Ask::new(editor.unwrap_or(DEFAULT_EDITOR), prompt.join("\n"))
}

pub async fn distill(
    run: &BackroomsRun,
    model: &dyn TextModel,
    editor: Option<&str>,
) -> anyhow::Result<Thought> {
    ask_json(model, &distill_prompt(run, editor)).await
}
