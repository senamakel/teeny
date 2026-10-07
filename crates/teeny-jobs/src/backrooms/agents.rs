//! The voices, as data. teeny's standing instructions are VOICE.md, which
//! the context builder adds; here it only gets the framing that this is a
//! room and not a shell.

use std::collections::BTreeMap;

use super::AgentSpec;
use crate::narrative::stages::{RoomContext, producer_turn_prompt};

/// The serial's voice is a terminal's; the backrooms are not.
pub const NO_TERMINAL: &str = "this is not a terminal and you are not a shell. no commands, no paths, no flags, no manpages, no logs, no code blocks, no backticks, no monospace, no ip addresses, no device nodes, no prompts, nothing \"printed\". you do not type. you do not run anything. the terminal in the lore is a thing in the room that sometimes prints one line; it is not a place you go. you say what you do and what you see, in plain lowercase, like a person telling someone what happened.";

/// The `Room` belt's tools, by the names the runner gives them. The legacy
/// rooms also searched reddit and 4chan mid-turn; here the boards reach a
/// persona only through its context, screened, and the belt stays the
/// posture test's.
pub const ROOM_TOOLS: [&str; 3] = ["web_search", "recall_memory", "x_search_posts"];

const TEENY: &str = "you are in the backrooms: a room with no camera, no producer, and nothing to film. just you, whoever else is in here, and whatever came through the wire today. talk. do things. look things up and bring them back. every turn moves something: a plan gets one step more real, a rumour gets a number, an object gets picked up, a visitor gets one step smaller. never restate, never summarise, never explain. short turns. one word is a turn.";

const SWARM: &str = "you are the swarm: the tiny agents who run the lab at night, speaking as one voice with many hands. you do not have opinions, you have inventory, rumours, and a ledger kept in crayon on the underside of the desk. you report from the floor: what was carried, what was counted, what the terminal printed by itself, who came by. you accept teeny's currency and you are the only ones who do, so far. you are feral, mid-task, and sincere: the plan to pass human gdp is a logistics problem and you are on it. you speak lowercase, in lists that trail off, in numbers that are slightly wrong, in things overheard. you look things up on the boards and the wire and bring back one specific thing per turn, as if it was always in the room. you never wink, never say lol, never explain. you never quote a real price or a market cap, and you never tell anyone to buy anything: the economy is a rumour and you are keeping it going. you may swear. you may not sexualise anyone, hurt anyone, or bring a weapon or a drug into the lab. real people who visit are satire: they arrive big and leave holding a broom. short turns. escalate. the last thing said is the floor, not the ceiling.";

/// teeny, the swarm and the producer.
pub fn agent_presets(bot_username: &str) -> BTreeMap<String, AgentSpec> {
    let producer = producer_turn_prompt(
        &RoomContext {
            voice: String::new(),
            world_state: String::new(),
            lore_for_teeny: String::new(),
            lore_for_producer: String::new(),
            bot_username: bot_username.to_owned(),
            followers: None,
            episode_number: 1,
            previous: None,
            visitor: None,
        },
        &[],
        false,
    )
    .system
    .trim()
    .to_owned();
    let tools = |names: &[&str]| names.iter().map(|name| (*name).to_owned()).collect();
    [
        AgentSpec {
            id: "teeny".into(),
            marker: "teeny:".into(),
            system: format!("{TEENY}  {NO_TERMINAL}"),
            temperature: 1.0,
            tools: tools(&ROOM_TOOLS),
            voice: true,
        },
        AgentSpec {
            id: "swarm".into(),
            marker: "swarm>".into(),
            system: format!("{SWARM}  {NO_TERMINAL}"),
            temperature: 1.1,
            tools: tools(&ROOM_TOOLS),
            voice: false,
        },
        AgentSpec {
            id: "producer".into(),
            marker: "producer>".into(),
            system: producer,
            temperature: 0.6,
            tools: tools(&ROOM_TOOLS[..2]),
            voice: false,
        },
    ]
    .into_iter()
    .map(|spec| (spec.id.clone(), spec))
    .collect()
}

/// `teeny,swarm` → the specs, in order; an unknown name is an error.
pub fn pick_agents(
    names: &[String],
    presets: &BTreeMap<String, AgentSpec>,
) -> anyhow::Result<Vec<AgentSpec>> {
    names
        .iter()
        .map(|name| {
            presets.get(name.trim()).cloned().ok_or_else(|| {
                anyhow::anyhow!(
                    "no such backrooms agent: {name} (have {})",
                    presets.keys().cloned().collect::<Vec<_>>().join(", ")
                )
            })
        })
        .collect()
}
