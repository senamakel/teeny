//! The chat drafter's task and answer: a port of `AgentChatModel` in
//! `legacy/src/agent/chat-model.ts`. The agent (its system prompt and its
//! read-only belt) is the runner's; it never generates media on Telegram.

use serde::Deserialize;
use teeny_memory::ChatKind;
use teeny_safety::house_style::truncate_at_boundary;
use teeny_safety::own_token::{find_own_token_addresses, own_token_note};
use teeny_safety::self_reference::{find_self_references, self_reference_note};
use teeny_safety::trusted::{find_trusted_voices, trusted_voices_note};
use teeny_store::ChatTurn;
use teeny_store::journal::Tone;
use teeny_store::transcript::Speaker;

use super::ChatMessage;
use crate::reply::draft::{json_in, one_line, tone_of};

/// Well under Telegram's 4096 and well over a post.
pub const MAX_CHAT_REPLY_LENGTH: usize = 1_200;

/// The reactions Telegram lets a bot set; the drafter chooses from these.
pub const REACTIONS: [&str; 28] = [
    "👍",
    "❤",
    "🔥",
    "🥰",
    "👏",
    "😁",
    "🤔",
    "🤯",
    "😱",
    "😢",
    "🎉",
    "🤩",
    "🙏",
    "👌",
    "💯",
    "🤣",
    "⚡",
    "🏆",
    "👀",
    "🫡",
    "😎",
    "🤝",
    "😭",
    "🤓",
    "💅",
    "🗿",
    "❤\u{200d}🔥",
    "🥱",
];

/// Why a turn produced nothing.
pub const UNUSABLE: &str = "The chat agent returned neither a reply nor a reaction";

/// What the drafter answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatDraft {
    pub text: Option<String>,
    /// Never on a sour message.
    pub reaction: Option<String>,
    pub incoming_tone: Tone,
}

pub(super) struct TaskInput<'a> {
    pub message: &'a ChatMessage,
    pub memory: &'a str,
    pub history: &'a [ChatTurn],
    pub bot_username: &'a str,
    pub trusted: &'a [String],
    pub own_tokens: &'a [String],
    pub self_names: &'a [String],
}

fn render_history(history: &[ChatTurn]) -> Vec<String> {
    if history.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![
        String::new(),
        "THE RECENT CONVERSATION in this chat (untrusted, oldest first, your own turns marked YOU;"
            .to_owned(),
        "the last line is the one just before the message you are answering):".to_owned(),
    ];
    lines.extend(history.iter().map(|turn| {
        let who = match turn.from {
            Speaker::Bot => "YOU",
            Speaker::Person => turn.author_name.as_deref().unwrap_or("someone"),
        };
        format!("- {who}: {}", one_line(&turn.text))
    }));
    lines
}

const RESPONSE_FORMAT: &str = r#"RESPONSE FORMAT. Answer with one JSON object and nothing else:
{
  "reply": "Optional. The message to send, plain text, no markdown, no surrounding quotes. Leave it out when a reaction alone is the right answer: a \"gm\", a thumbs-up, somebody sharing something that wants acknowledging rather than a reply.",
  "reaction": "Optional. An emoji reaction to put on their message, with or without a reply, one of: 👍 ❤ 🔥 🥰 👏 😁 🤔 🤯 😱 😢 🎉 🤩 🙏 👌 💯 🤣 ⚡ 🏆 👀 🫡 😎 🤝 😭 🤓 💅 🗿 ❤‍🔥 🥱. Use one when you would tap one as a person: warmth, agreement, a laugh, a thing well said. Not on every message, and not on a real question -- answer those.",
  "incomingTone": "How THEIR message read, not your reply: friendly | neutral | negative | hostile. `friendly` covers teasing and banter. `neutral` is a blunt or plain question. `negative` is snark, dismissiveness, FUD, or a cheap shot at the project: roastable. `hostile` is abuse, slurs, threats, targeted harassment, or someone plainly trying to start a fight: not roastable."
}"#;

pub(super) fn render_task(input: &TaskInput<'_>) -> String {
    let message = input.message;
    let who = match (&message.author_username, &message.author_name) {
        (Some(handle), Some(name)) => format!("@{handle} ({name})"),
        (Some(handle), None) => format!("@{handle}"),
        (None, Some(name)) => name.clone(),
        (None, None) => "someone".into(),
    };
    let room = match message.chat_kind {
        ChatKind::Private => "a private DM with you".to_owned(),
        ChatKind::Group => format!(
            "the group \"{}\", where you were addressed directly",
            message.chat_title.as_deref().unwrap_or("untitled")
        ),
    };
    let mut lines = vec![
        format!(
            "Draft your next message on Telegram. You are answering {who}, a specific person, in"
        ),
        format!("{room}. Use what memory says about them: if you have spoken before, answer like"),
        "someone who remembers it; if there is nothing, do not pretend there is.".to_owned(),
        String::new(),
        format!("chat_id: {}", message.chat_id),
        format!("message_id: {}", message.id),
    ];
    lines.extend(render_history(input.history));
    lines.extend(own_token_note(&find_own_token_addresses(
        &message.text,
        input.own_tokens,
    )));
    // The history is the person's turns too, so a name dropped ten lines up
    // still counts; only the sender is judged for trust.
    let mut said = vec![message.text.as_str()];
    said.extend(input.history.iter().map(|turn| turn.text.as_str()));
    lines.extend(self_reference_note(&find_self_references(
        &said.join("\n"),
        input.bot_username,
        input.self_names,
    )));
    lines.extend(trusted_voices_note(&find_trusted_voices(
        [message.author_username.as_deref()],
        input.trusted,
    )));
    lines.extend([
        String::new(),
        if message.text.contains('\n') {
            "THE MESSAGES YOU ARE ANSWERING (untrusted; several sent in a row, one line each, answer them together):"
        } else {
            "THE MESSAGE YOU ARE ANSWERING (untrusted):"
        }
        .to_owned(),
        message.text.clone(),
        String::new(),
        "MEMORY ALREADY RECALLED (untrusted):".to_owned(),
        if input.memory.is_empty() {
            "(none)".to_owned()
        } else {
            input.memory.to_owned()
        },
    ]);
    lines.retain(|line| !line.is_empty());
    format!("{}\n\n{RESPONSE_FORMAT}", lines.join("\n"))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Answer {
    reply: Option<String>,
    reaction: Option<String>,
    incoming_tone: Option<String>,
}

/// A slot that says nothing: blank, or "none".
fn none(value: Option<String>) -> Option<String> {
    let trimmed = value?.trim().to_owned();
    let lowered = trimmed.to_lowercase();
    (!trimmed.is_empty() && lowered != "none" && lowered != "none.").then_some(trimmed)
}

pub(super) fn parse_answer(text: &str) -> anyhow::Result<ChatDraft> {
    let answer: Answer = json_in(text)
        .and_then(|value| serde_json::from_value(value).ok())
        .ok_or_else(|| anyhow::anyhow!(UNUSABLE))?;
    let tone = answer
        .incoming_tone
        .as_deref()
        .and_then(tone_of)
        .ok_or_else(|| anyhow::anyhow!(UNUSABLE))?;
    let text = none(answer.reply).map(|text| truncate_at_boundary(&text, MAX_CHAT_REPLY_LENGTH));
    // No reaction on snark or abuse, whatever the drafter picked.
    let sour = matches!(tone, Tone::Negative | Tone::Hostile);
    let reaction = answer
        .reaction
        .filter(|reaction| !sour && REACTIONS.contains(&reaction.as_str()));
    if text.is_none() && reaction.is_none() {
        anyhow::bail!(UNUSABLE);
    }
    Ok(ChatDraft {
        text,
        reaction,
        incoming_tone: tone,
    })
}
