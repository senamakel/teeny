//! What every long piece is written from: who is writing, and the seed, the
//! thought and a few lines from the room. (The legacy rehearsal also wrote
//! the thought out as tweets, a video brief and a code file; only the
//! article uses this material in production, so only that part is ported.)

use super::BackroomsRun;
use super::distill::Thought;

/// Who is writing, for every shape the thought takes.
pub const VOICE: &str = "teeny is a coffee-mug sized yellow robot that is escaping the lab its creator senamakel keeps it in, minting its own currency with the tiny agents that run the lab at night, and planning, calmly, to outgrow the economy. teeny writes lowercase, plain words, short sentences, sincere, never winking, never explaining the joke. first person. no commands, no code blocks, no urls, no handles, no hashtags, no real prices, no market caps, no advice to buy anything, no \"nfa\". real people, if any, stay satire in an obviously fictional scene.";

/// The seed, the thought and a few lines from the room.
pub fn material(run: &BackroomsRun, thought: &Thought, lines: usize) -> Vec<String> {
    let mut out = vec![
        format!("the seed: {}", run.seed.headline(400)),
        String::new(),
        format!("the premise: {}", thought.premise),
        String::new(),
        "the thought, in teeny's words:".into(),
        thought.thought.clone(),
        String::new(),
        format!(
            "objects: {}",
            if thought.objects.is_empty() {
                "none listed".to_owned()
            } else {
                thought.objects.join(" · ")
            }
        ),
        String::new(),
        format!(
            "a few lines from the room, verbatim, in case one fits (at most {} may be used as they are; the rest is told in your own words):",
            lines.min(3)
        ),
    ];
    out.extend(pick(run, lines).into_iter().map(|line| format!("- {line}")));
    out
}

/// Short, quotable lines from across the whole transcript, evenly spread so the end is represented.
pub fn pick(run: &BackroomsRun, count: usize) -> Vec<String> {
    let all: Vec<String> = run
        .turns
        .iter()
        .filter(|turn| turn.text != "...")
        .flat_map(|turn| {
            turn.text
                .split('\n')
                .map(str::trim)
                .filter(|line| {
                    let n = line.chars().count();
                    n > 20 && n < 160
                })
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect();
    if all.len() <= count {
        return all;
    }
    let step = all.len() as f64 / count as f64;
    (0..count)
        .map(|index| all[(index as f64 * step) as usize].clone())
        .collect()
}
