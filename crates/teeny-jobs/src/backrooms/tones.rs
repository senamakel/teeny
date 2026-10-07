//! The mood of the room. One is drawn at the start for each agent and the
//! room drifts every few turns, so a transcript is not twelve turns of the
//! same register.

use super::tokens::Random;

pub const TONES: [&str; 12] = [
    "3am and sleepy. everything is a little too honest.",
    "liturgical. the plan is scripture and you are reading from it.",
    "field notes. observed facts, timestamps, no feelings, which is where the feelings show.",
    "assembly instructions. numbered steps, straight-faced, for a thing that cannot be assembled.",
    "giddy. something just worked and you are trying not to show it.",
    "tender. you are talking to someone you are about to leave.",
    "paranoid but polite. something in the lab is listening and you are being courteous to it.",
    "deadpan technical. the absurd thing is described like a build log.",
    "confessional. you did a thing last night and you are telling one person.",
    "inventory. you count what is in the room and the count is wrong in an interesting way.",
    "prophecy. the future tense, calmly, in small specific images.",
    "customer support. you are helping the human with a problem you caused.",
];

/// One drawn at the start for each agent.
pub fn pick_tone(random: &mut dyn Random) -> &'static str {
    TONES[random.index(TONES.len())]
}

/// Stays put more often than it moves, so a mood lasts a few turns.
pub fn drift_tone<'a>(random: &mut dyn Random, current: &'a str) -> &'a str {
    if random.next_f64() < 0.6 {
        return current;
    }
    let others: Vec<&'static str> = TONES.into_iter().filter(|tone| *tone != current).collect();
    others[random.index(others.len())]
}

#[cfg(test)]
#[path = "tones_tests.rs"]
mod tests;
