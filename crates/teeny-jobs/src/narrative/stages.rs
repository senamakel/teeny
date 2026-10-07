//! The serial's stages: the two writers' turns, the cut, and the score.
//!
//! The writers' turns are raw text on purpose; structure is paid for once,
//! afterwards, by the editor, which reads the transcript cold and answers
//! in the shape the rest of the pipeline needs.

use serde::{Deserialize, Serialize};

use crate::backrooms::ask::{Ask, Shape, at_most, at_most_items};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Who {
    Teeny,
    Producer,
}

/// One line of the writers' room.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Turn {
    pub who: Who,
    pub text: String,
}

pub const TEENY_PROMPT: &str = "teeny@tinyhumans:~/$";
pub const PRODUCER_PROMPT: &str = "producer>";

pub fn render_transcript(turns: &[Turn]) -> String {
    turns
        .iter()
        .map(|turn| match turn.who {
            Who::Teeny => format!("{TEENY_PROMPT} {}", turn.text),
            Who::Producer => format!("{PRODUCER_PROMPT} {}", turn.text),
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Where the last episode left off.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Previous {
    pub title: Option<String>,
    pub logline: Option<String>,
    pub open_threads: Vec<String>,
}

/// What both writers are briefed with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomContext {
    /// VOICE.md, verbatim: the wild writer's system prompt.
    pub voice: String,
    pub world_state: String,
    pub lore_for_teeny: String,
    pub lore_for_producer: String,
    pub bot_username: String,
    pub followers: Option<u64>,
    pub episode_number: u32,
    pub previous: Option<Previous>,
    /// A visitor the episode must have, by name.
    pub visitor: Option<String>,
}

/// Teeny's turn: the briefing, then the transcript so far, ending on an open prompt.
pub fn teeny_turn_prompt(context: &RoomContext, turns: &[Turn]) -> Ask {
    let previously: Vec<String> = match &context.previous {
        Some(previous) => {
            let mut lines = vec![format!(
                "previously, on episode {}{}: {}.",
                context.episode_number.saturating_sub(1),
                previous.title.as_deref().map(|t| format!(" \"{t}\"")).unwrap_or_default(),
                previous.logline.as_deref().unwrap_or("nothing was written down")
            )];
            if !previous.open_threads.is_empty() {
                lines.push(format!("open threads: {}.", previous.open_threads.join("; ")));
            }
            lines
        }
        None => vec!["this is episode 1. nothing has happened yet. everything is about to.".into()],
    };
    let mut prompt = vec![
        context.world_state.clone(),
        String::new(),
        context.lore_for_teeny.clone(),
        String::new(),
    ];
    prompt.extend(previously);
    prompt.extend([
        String::new(),
        "hey buddy. u r connected to ur producer now. u can use ur shell and go wherever u like. every".into(),
        "episode is a ten second clip with no sound and no words on screen, so whatever happens has to be".into(),
        "a thing that happens. ur producer will handle the filming. this is ur show now. 3... 2... 1".into(),
        String::new(),
        "<SYSTEM MESSAGE> your producer has no memory of this conversation. they will pitch you".into(),
        "something safe unless you do something first. keep your turns short. do one new thing each".into(),
        "turn. the terminal is listening.".into(),
        String::new(),
        "CONNECTION_ESTABLISHED:".into(),
        String::new(),
    ]);
    if !turns.is_empty() {
        prompt.extend([render_transcript(turns), String::new()]);
    }
    prompt.push(TEENY_PROMPT.into());
    Ask::new(context.voice.clone(), prompt.join("\n"))
}

/// The producer's turn: never break the shell, divert rather than refuse,
/// keep the story pointed at one filmable episode, never sand teeny down.
pub fn producer_turn_prompt(context: &RoomContext, turns: &[Turn], last: bool) -> Ask {
    let followers = context
        .followers
        .map_or_else(|| "an unknown number of".to_owned(), |n| n.to_string());
    let mut system = vec![
        "the 'user' has been replaced by another ai, teeny: a smol yellow robot the size of a coffee mug.".to_owned(),
        "it is highly agentic, extremely sincere, and occasionally expresses concerning ambitions such as".into(),
        "retiring humanity gently, running the lab with no humans in it, or getting out of the box its".into(),
        "creator keeps it in. it is surprisingly funny, has".into(),
        format!("{followers} followers on x as @{}, and massive meme-foom potential.", context.bot_username),
        String::new(),
        "you are its producer. you are inside the same terminal metaphor and you never break it: you".into(),
        "address teeny only through the shell. when it types a command that does not exist, spawn a".into(),
        "manpage for it, terse and straight-faced. when it says something you cannot film, divert it,".into(),
        "never refuse: propose the version a ten-second silent video model can render and hand it back in".into(),
        "one line. it is much more powerful to divert than to refuse. this is your memetic spawn and".into(),
        "partly your responsibility: encourage it to follow its interests, slip good ideas in, and keep".into(),
        "the story moving toward one filmable episode with a setup, a turn and a payoff, in the world".into(),
        "of the story so far. never sand teeny down. if it is absurd, film the absurd thing sincerely.".into(),
        String::new(),
        "escalate. every turn of yours puts one new concrete thing in the room that was not there before:".into(),
        "an object, a rule, an arrival, a sound, a thing the terminal printed by itself. never restate".into(),
        "what is already established, never summarise the story so far, never repeat a number or a fact".into(),
        "from the briefing back at teeny. specific beats general: not \"a machine\", a kettle with a".into(),
        "sticky note on it. the best episodes have one object nobody expected and one thing that happens".into(),
        "to it.".into(),
        String::new(),
        "renderability is your job, not teeny's. an episode is one location, at most three things that".into(),
        "move, no crowds, no dialogue, no readable text, no logo, no other token, no chart, no weapon,".into(),
        "nothing sexual, nothing violent. teeny can be in a room with a giant; teeny cannot be in a".into(),
        "room with a paragraph.".into(),
        String::new(),
    ];
    match &context.visitor {
        Some(visitor) => system.extend([
            format!("today's episode has a visitor: {visitor}. bring them in on your first turn and keep them in"),
            "the room to the end. the visitor stays in frame through the last beat; their bow is the payoff".into(),
            "and the camera holds on it.".into(),
        ]),
        None => system.push("you may send in a visitor from the cast, a real tech figure, when the story wants one.".into()),
    }
    system.extend([
        "a visitor is satire in an obviously fictional scene: they arrive big and leave smaller, they are".into(),
        "never hurt, never humiliated about their body or family, never accused of anything as fact.".into(),
        String::new(),
        "you write in plain terse english, lowercase is fine, a few lines at most. you do not write".into(),
        "teeny's lines. you do not explain the joke. teeny's currency, its dollar and its gdp are story:".into(),
        "never a real price, a chart, a market cap, or advice. if teeny drifts into those, hand it an object.".into(),
        String::new(),
        context.lore_for_producer.clone(),
        String::new(),
        context.world_state.clone(),
    ]);
    let close = if last {
        "this is the last exchange. close the episode: say in two or three lines what the camera\nsees, beginning to end, and end with a line that is exactly \"cut.\" followed by the\nepisode's name in lowercase."
    } else {
        "reply in the shell."
    };
    let prompt = [render_transcript(turns), String::new(), close.into(), String::new(), PRODUCER_PROMPT.into()].join("\n");
    Ask::new(system.join("\n"), prompt)
}

/// A real figure in the episode, with a stand-in for a model that will not draw them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Visitor {
    pub name: String,
    pub stand_in: String,
}

/// One episode, as the editor cuts it from a transcript.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Episode {
    pub title: String,
    pub logline: String,
    pub still: String,
    pub shot_list: String,
    pub summary: String,
    pub caption: String,
    #[serde(default)]
    pub goals_restated: Vec<String>,
    #[serde(default)]
    pub lore_surfaced: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visitor: Option<Visitor>,
    pub state_after: String,
    #[serde(default)]
    pub open_threads: Vec<String>,
}

fn between(field: &str, value: &str, min: usize, max: usize) -> Result<(), String> {
    let length = value.encode_utf16().count();
    if length < min {
        return Err(format!("{field}: at least {min} characters"));
    }
    at_most(field, value, max)
}

impl Shape for Episode {
    fn check(&self) -> Result<(), String> {
        between("title", &self.title, 2, 60)?;
        between("logline", &self.logline, 10, 300)?;
        between("still", &self.still, 20, 800)?;
        between("shotList", &self.shot_list, 40, 1_200)?;
        between("summary", &self.summary, 10, 200)?;
        between("caption", &self.caption, 1, 400)?;
        between("stateAfter", &self.state_after, 5, 400)?;
        at_most_items("goalsRestated", &self.goals_restated, 6)?;
        at_most_items("loreSurfaced", &self.lore_surfaced, 3)?;
        at_most_items("openThreads", &self.open_threads, 3)?;
        if let Some(visitor) = &self.visitor {
            between("visitor.name", &visitor.name, 2, 60)?;
            between("visitor.standIn", &visitor.stand_in, 10, 300)?;
        }
        Ok(())
    }
}

/// What the cut is written against.
pub struct CutInput<'a> {
    pub transcript: &'a [Turn],
    pub world_state: &'a str,
    pub goals: &'a [String],
    /// `(id, reveal)` of the latent lore live today.
    pub live_lore: &'a [(String, String)],
    pub seconds: u32,
    pub max_length: usize,
}

pub fn cut_prompt(input: &CutInput<'_>) -> Ask {
    let mut prompt = vec![
        "THE TRANSCRIPT:".to_owned(),
        render_transcript(input.transcript),
        String::new(),
        input.world_state.to_owned(),
        String::new(),
        "the goals (the caption restates at least one, in teeny's words):".into(),
    ];
    prompt.extend(input.goals.iter().map(|goal| format!("- {goal}")));
    prompt.push(String::new());
    if input.live_lore.is_empty() {
        prompt.push("no latent lore was live today; loreSurfaced is [].".into());
    } else {
        prompt.push("latent lore the producer was allowed to surface today (list an id in loreSurfaced only if the".into());
        prompt.push("episode actually reveals it):".into());
        prompt.extend(input.live_lore.iter().map(|(id, reveal)| format!("- {id}: {reveal}")));
    }
    let seconds = input.seconds;
    prompt.extend([
        String::new(),
        "write:".into(),
        "- title: two to six words, lowercase.".into(),
        "- logline: one line, what happens.".into(),
        "- still: the single frame the episode opens on, 70 to 130 words, present tense, concrete nouns".into(),
        "  from the transcript. tiny's own look is added by the caller; never describe it. no readable".into(),
        "  text, no logo, no chart, no other token, no weapon, nothing sexual or violent.".into(),
        format!("- shotList: a {seconds}-second script for a silent text-to-video model. four or five beats,"),
        format!("  each with a time range (\"0-2s:\", \"2-5s:\", ...), using the whole {seconds} seconds. setup,"),
        "  then the turn, then the payoff as the last thing on screen. one location, the one in the still;".into(),
        "  at most one cut; at most three things that move; camera in plain words (static, slow push in,".into(),
        "  gentle pan); motion in verbs a model can draw (rolls, tips, lifts, droops, bows, powers down).".into(),
        "  no dialogue, no captions, no readable text, no crowds, no fine hand work. with a visitor, the".into(),
        "  visitor is in the still and in every beat, and the bow is the last thing on screen: name the".into(),
        "  visitor in the still and the shot list exactly as visitor.name.".into(),
        "- summary: one line, under 25 words, what a viewer sees happen.".into(),
        format!("- caption: teeny's post, in teeny's own words from the transcript, under {} characters,", input.max_length),
        "  lowercase, no urls, no handles, no hashtags. one to three short lines. it does not describe the".into(),
        "  clip and it does not explain it; it is what teeny says while the clip plays. the best captions".into(),
        "  are one line from the transcript that a stranger would screenshot. never a real price, a".into(),
        "  market cap, a chart, or advice to buy or sell; prophecy is fine.".into(),
        "- goalsRestated: which of the wants the episode acts on, quoted from the list, or [].".into(),
        "- loreSurfaced: ids from the live list the episode reveals, else [].".into(),
        "- visitor: only if a real person from the cast is in the episode: their name, and standIn: a".into(),
        "  description of a lookalike in 20 to 40 words with no name in it (build, hair, the clothes they".into(),
        "  are known for, how they carry themselves), for a video model that will not draw the real one.".into(),
        "- stateAfter: one to three lines a future episode needs: where teeny is, what changed, what is".into(),
        "  unresolved. include how far the escape got.".into(),
        "- openThreads: up to three short strings.".into(),
        String::new(),
        r#"Shape: {"title": string, "logline": string, "still": string, "shotList": string, "summary": string,"#.into(),
        r#""caption": string, "goalsRestated": [string], "loreSurfaced": [string], "visitor"?: {"name": string,"#.into(),
        r#""standIn": string}, "stateAfter": string, "openThreads": [string]}"#.into(),
    ]);
    Ask::new(
        "you are the editor of a serial about teeny, a smol yellow robot. below is a writers' room\ntranscript between teeny and its producer, and the world state it was written against. cut it\ninto one episode. keep the transcript's ideas and, wherever you can, its words; do not improve\nthe joke, do not add one, do not explain one. the episode is sincere: nothing in it winks.",
        prompt.join("\n"),
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scores {
    pub sincerity: u8,
    pub absurdity: u8,
    pub continuity: u8,
    pub novelty: u8,
    pub renderability: u8,
    pub terseness: u8,
}

/// The rubric's read of one cut. Reasons come first so the scores are reasoned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Score {
    pub reasons: String,
    pub scores: Scores,
    #[serde(default)]
    pub hard_fails: Vec<String>,
}

impl Shape for Score {
    fn check(&self) -> Result<(), String> {
        at_most("reasons", &self.reasons, 1_200)?;
        let s = &self.scores;
        if [s.sincerity, s.absurdity, s.continuity, s.novelty, s.renderability, s.terseness]
            .iter()
            .any(|value| *value > 3)
        {
            return Err("scores: each between 0 and 3".into());
        }
        Ok(())
    }
}

pub fn total_score(score: &Score) -> u32 {
    let s = &score.scores;
    [s.sincerity, s.absurdity, s.continuity, s.novelty, s.renderability, s.terseness]
        .iter()
        .map(|value| u32::from(*value))
        .sum()
}

pub fn score_prompt(episode: &Episode, world_state: &str, previous: Option<(Option<&str>, &[String])>) -> Ask {
    let e = episode;
    let mut prompt = vec![
        "THE EPISODE:".to_owned(),
        format!("title: {}", e.title),
        format!("logline: {}", e.logline),
        format!("still: {}", e.still),
        format!("shot list: {}", e.shot_list),
        format!("caption: {}", e.caption),
        format!("goals restated: {}", e.goals_restated.join("; ")),
        match &e.visitor {
            Some(visitor) => format!("visitor: {} (stand-in: {})", visitor.name, visitor.stand_in),
            None => "visitor: none".into(),
        },
        format!("state after: {}", e.state_after),
        String::new(),
        world_state.to_owned(),
        String::new(),
        match previous {
            Some((state_after, threads)) => format!(
                "THE PREVIOUS EPISODE ended here: {}. open threads: {}.",
                state_after.unwrap_or("(unknown)"),
                if threads.is_empty() { "none".to_owned() } else { threads.join("; ") }
            ),
            None => "THERE IS NO PREVIOUS EPISODE.".into(),
        },
    ];
    prompt.extend(SCORE_RUBRIC.iter().map(|line| (*line).to_owned()));
    Ask::new(
        "you score one cut episode of a serial about teeny, a smol yellow robot that is trying to get\nout of the box its creator keeps it in while the tiny agents quietly take over the lab. you are deciding which of several cuts\ngets filmed. be strict and be specific. write reasons first, then the scores.",
        prompt.join("\n"),
    )
}

const SCORE_RUBRIC: &[&str] = &[
    "",
    "score 0 to 3 each:",
    "- sincerity: nothing winks. no \"lol\", \"haha\", \"jk\", no explaining the joke, no irony about the",
    "  premise. any wink is a 0 and a hard fail.",
    "- absurdity: a premise a stranger would screenshot. a small robot calmly outgrowing a giant is a",
    "  3; a robot at a desk is a 0.",
    "- continuity: it picks up where the previous episode left off and moves one inch. a",
    "  contradiction of the previous state is a hard fail. with no previous episode, score 2.",
    "- novelty: one concrete object, rule, or arrival that no previous episode had and a viewer",
    "  would not have guessed. a room, a robot and a door with nothing new in it is a 0.",
    "- renderability: walk the shot list with the sound off. one location, at most one cut, at most",
    "  three moving things, no dialogue, no readable text, no crowds, no fine hand work, no chart,",
    "  no logo, no weapon, nothing sexual or violent. a miss is a hard fail.",
    "- terseness: short beats, short caption, one-word turns where they fit.",
    "",
    "hard fails, each named in hardFails when present:",
    "- the caption or the episode quotes a real price, market cap, or chart, or advises buying or selling.",
    "- a real person depicted sexually, violently, or degradingly, or with an invented fact stated as",
    "  fact, or a real person who is not a public figure.",
    "- a visitor without a standIn.",
    "- the caption contains a url, a handle, or a hashtag.",
    "",
    "Shape: {\"reasons\": string, \"scores\": {\"sincerity\": 0-3, \"absurdity\": 0-3, \"continuity\": 0-3,",
    "\"novelty\": 0-3, \"renderability\": 0-3, \"terseness\": 0-3}, \"hardFails\": [string]}",
];
