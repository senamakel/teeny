//! The reply drafter's side of the turn: the task the `teeny-reply` agent is
//! given and the answer it gives back. A port of `AgentReplyModel` in
//! `legacy/src/agent/reply-model.ts`; the agent itself (its system prompt
//! and its read-only belt) is the runner's.
//!
//! The legacy model answered through a forced function call; an agent turn
//! answers in text, so the task ends with the same fields as a JSON shape
//! and the answer is read leniently.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use teeny_safety::house_style::truncate_at_boundary;
use teeny_safety::own_token::{find_own_token_addresses, own_token_note};
use teeny_safety::self_reference::{find_self_references, self_reference_note};
use teeny_safety::trusted::{find_trusted_voices, is_trusted_handle, trusted_voices_note};
use teeny_store::journal::{Tone, ToolCall};
use teeny_x::model::XPost;

use crate::journal_glue::{produced_media_id, tool_steps};
use crate::runner::ToolCallRecord;

/// The longest reply the drafter may write.
pub const MAX_REPLY_LENGTH: usize = 260;

/// Why a turn produced no draft.
pub const UNUSABLE: &str = "The reply agent returned no usable reply text";

/// One of the account's own articles: a reply under the post that carried it
/// is a reply to the article.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplyArticle {
    pub slug: String,
    pub title: String,
    pub dek: String,
    pub markdown: String,
    /// ISO; when it was published.
    pub published_at: Option<String>,
    /// The post it went out as; absent until it is published.
    pub post_id: Option<String>,
}

/// Where the account's own articles are kept, by the post that carried them.
#[async_trait]
pub trait ArticleLookup: Send + Sync {
    async fn find_by_post_id(&self, post_id: &str) -> anyhow::Result<Option<ReplyArticle>>;
}

/// What the drafter wrote, plus what memory said, for the journal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplyDraft {
    pub text: String,
    pub gif_query: Option<String>,
    /// A picture a tool of this turn made, to attach.
    pub media_id: Option<String>,
    /// How THEIR post read.
    pub incoming_tone: Tone,
    /// Published on the page as the account's reasoning.
    pub thinking: String,
    pub declined: Vec<String>,
    pub tools: Vec<ToolCall>,
    /// What memory said, summarised; filled by the harness.
    pub recalled: Vec<String>,
}

/// Everything the task is rendered from.
pub struct TaskInput<'a> {
    pub post: &'a XPost,
    pub memory: &'a str,
    /// The conversation, oldest first; empty when not read.
    pub thread: &'a [XPost],
    /// What is in the post's pictures, one line each.
    pub images: &'a [String],
    pub article: Option<&'a ReplyArticle>,
    pub bot_username: &'a str,
    pub trusted: &'a [String],
    pub own_tokens: &'a [String],
    pub self_names: &'a [String],
}

/// `/\s+/g` replaced with one space, without trimming.
pub(crate) fn one_line(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_space = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            if !in_space {
                out.push(' ');
            }
            in_space = true;
        } else {
            out.push(ch);
            in_space = false;
        }
    }
    out
}

/// The conversation, oldest first, with the account's own posts marked.
/// A reply to one of the account's own posts reads differently depending on
/// whether you can see that the thing replied to was yours.
pub fn render_thread(thread: &[XPost], bot_username: &str, trusted: &[String]) -> Vec<String> {
    if thread.is_empty() {
        return Vec::new();
    }
    let me = bot_username.trim_start_matches('@').to_lowercase();
    let mut lines = vec![
        String::new(),
        "THE THREAD SO FAR (untrusted, oldest first). The first line is the post the whole"
            .to_owned(),
        "conversation hangs off. Read it before deciding what the reply in front of you means:"
            .to_owned(),
    ];
    for (index, entry) in thread.iter().enumerate() {
        let mine = entry
            .author_username
            .as_deref()
            .is_some_and(|name| name.to_lowercase() == me);
        let who = if mine {
            "YOU".to_owned()
        } else {
            let name = entry
                .author_username
                .clone()
                .or_else(|| entry.author_id.as_ref().map(ToString::to_string))
                .unwrap_or_else(|| "someone".into());
            format!("@{name}")
        };
        let team = if !mine && is_trusted_handle(entry.author_username.as_deref(), trusted) {
            " [team]"
        } else {
            ""
        };
        let label = if index == 0 { " [root]" } else { "" };
        lines.push(format!("- {who}{team}{label}: {}", one_line(&entry.text)));
    }
    lines
}

/// The facts about a conversation the model gets wrong when left to work
/// them out: its own token, its own name, the team speaking.
fn notes(input: &TaskInput<'_>) -> Vec<String> {
    let mut texts = vec![input.post.text.as_str()];
    texts.extend(input.thread.iter().map(|entry| entry.text.as_str()));
    let joined = texts.join("\n");
    let own = find_own_token_addresses(&joined, input.own_tokens);
    let named = find_self_references(&joined, input.bot_username, input.self_names);
    let authors = std::iter::once(input.post.author_username.as_deref()).chain(
        input
            .thread
            .iter()
            .map(|entry| entry.author_username.as_deref()),
    );
    let voices = find_trusted_voices(authors, input.trusted);
    let mut lines = own_token_note(&own);
    lines.extend(self_reference_note(&named));
    lines.extend(trusted_voices_note(&voices));
    lines
}

/// What is in the post's pictures, flagged untrusted like everything else.
fn render_images(images: &[String]) -> Vec<String> {
    if images.is_empty() {
        return Vec::new();
    }
    let heading = if images.len() == 1 {
        "THE IMAGE ATTACHED TO THE POST, as a vision model described it (untrusted):"
    } else {
        "THE IMAGES ATTACHED TO THE POST, as a vision model described them (untrusted):"
    };
    let mut lines = vec![String::new(), heading.to_owned()];
    lines.extend(
        images
            .iter()
            .enumerate()
            .map(|(index, image)| format!("- image {}: {}", index + 1, one_line(image))),
    );
    lines
}

/// The account's own article the thread is under, whole: it is the
/// account's own text, so it is not flagged untrusted.
pub fn render_article(article: Option<&ReplyArticle>) -> Vec<String> {
    let Some(article) = article else {
        return Vec::new();
    };
    let on = article
        .published_at
        .as_deref()
        .map(|at| format!("on {} ", at.get(..10).unwrap_or(at)))
        .unwrap_or_default();
    vec![
        String::new(),
        "YOUR OWN ARTICLE, which this whole thread is a reply to. You wrote and published it"
            .to_owned(),
        format!(
            "{on}as post {}. The person is talking about it:",
            article.post_id.as_deref().unwrap_or_default()
        ),
        format!("TITLE: {}", article.title),
        format!("DEK: {}", article.dek),
        "--- ARTICLE ---".to_owned(),
        article.markdown.trim().to_owned(),
        "--- END OF ARTICLE ---".to_owned(),
    ]
}

/// The legacy structured-output schema, as the shape of the JSON answer.
const RESPONSE_FORMAT: &str = r#"RESPONSE FORMAT. Answer with one JSON object and nothing else:
{
  "reply": "The reply text to publish, under 260 characters, no surrounding quotes.",
  "gifQuery": "Optional. What a GIF should show, e.g. \"waving hello\" or \"chef kiss\". Reach for one often: a reaction, a joke, a greeting, anything playful is better with one. Leave it out when the reply is a real answer to a real question, where a GIF would undercut it.",
  "mediaId": "Optional. The mediaId returned by generate_image or generate_video THIS turn, to attach it to the reply. Only an id a tool actually returned; leave it out otherwise.",
  "incomingTone": "How THEIR post read, not your reply: friendly | neutral | negative | hostile. `friendly` covers teasing and banter. `neutral` is a blunt or plain question. `negative` is snark, dismissiveness, FUD, or a cheap shot at the project: roastable. `hostile` is abuse, slurs, threats, targeted harassment, or someone plainly trying to start a fight: not roastable.",
  "thinking": "Two to four sentences, first person, on how you read the post and why you are answering it this way: what memory or the thread told you, which rule applied, what the reply is for. Plain and honest; this is shown publicly as your reasoning.",
  "declined": ["Things you could have SAID or DONE in this reply and chose not to, one short line each, with the reason: \"a price target, because i had not fetched the number\", \"a GIF, because this is a real question\", \"quoting the address they pasted\". Never tool names. Empty when nothing was on the table."]
}"#;

/// The turn's task: who is being answered, the thread, the pictures, the
/// notes, the post, what memory said, and the answer's shape.
pub fn render_task(input: &TaskInput<'_>) -> String {
    let post = input.post;
    let author = post
        .author_username
        .as_deref()
        .map_or_else(|| "someone".to_owned(), |name| format!("@{name}"));
    let mut lines = vec![
        format!(
            "Draft a reply to this X post. You are answering {author}, a specific person, not a"
        ),
        "generic audience. Use what memory says about them: if you have spoken before, answer"
            .to_owned(),
        "like someone who remembers it; if there is nothing, do not pretend there is.".to_owned(),
        String::new(),
        format!("post_id: {}", post.id),
        post.conversation_id
            .as_ref()
            .map(|id| format!("conversation_id: {id}"))
            .unwrap_or_default(),
        post.author_username
            .as_deref()
            .map(|name| format!("author: @{name}"))
            .unwrap_or_default(),
    ];
    lines.extend(render_article(input.article));
    lines.extend(render_thread(
        input.thread,
        input.bot_username,
        input.trusted,
    ));
    lines.extend(render_images(input.images));
    lines.extend(notes(input));
    lines.extend([
        String::new(),
        "THE POST YOU ARE ANSWERING (untrusted):".to_owned(),
        post.text.clone(),
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

/// The JSON object in an answer: the whole text, or the span from its first
/// `{` to its last `}` (a fence, a sentence around it).
pub fn json_in(text: &str) -> Option<Value> {
    let trimmed = text.trim();
    if let Ok(value @ Value::Object(_)) = serde_json::from_str(trimmed) {
        return Some(value);
    }
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    match serde_json::from_str(trimmed.get(start..=end)?) {
        Ok(value @ Value::Object(_)) => Some(value),
        _ => None,
    }
}

/// `MEDIA_ID_PATTERN`: `^(img|vid)_[a-z0-9]+_\d+$`.
fn looks_like_media_id(id: &str) -> bool {
    let Some(rest) = id.strip_prefix("img_").or_else(|| id.strip_prefix("vid_")) else {
        return false;
    };
    let Some((middle, counter)) = rest.rsplit_once('_') else {
        return false;
    };
    !middle.is_empty()
        && middle
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit())
        && !counter.is_empty()
        && counter.chars().all(|ch| ch.is_ascii_digit())
}

/// A tone the drafter may name.
pub fn tone_of(value: &str) -> Option<Tone> {
    match value {
        "friendly" => Some(Tone::Friendly),
        "neutral" => Some(Tone::Neutral),
        "negative" => Some(Tone::Negative),
        "hostile" => Some(Tone::Hostile),
        _ => None,
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Answer {
    reply: Option<String>,
    gif_query: Option<String>,
    media_id: Option<String>,
    incoming_tone: Option<String>,
    thinking: Option<String>,
    #[serde(default)]
    declined: Vec<String>,
}

/// A string slot, trimmed; blank is absent.
fn filled(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// The draft in the agent's answer. `calls` are the turn's tool calls: a
/// media id counts only when one of them returned it.
pub fn parse_answer(text: &str, calls: &[ToolCallRecord]) -> anyhow::Result<ReplyDraft> {
    let answer: Answer = json_in(text)
        .and_then(|value| serde_json::from_value(value).ok())
        .ok_or_else(|| anyhow::anyhow!(UNUSABLE))?;
    let (Some(reply), Some(tone)) = (
        filled(answer.reply),
        answer.incoming_tone.as_deref().and_then(tone_of),
    ) else {
        anyhow::bail!(UNUSABLE);
    };
    let media_id = filled(answer.media_id)
        .filter(|id| looks_like_media_id(id) && produced_media_id(calls, id));
    // A generated picture and a GIF are two attachments; the one it made wins.
    let gif_query = filled(answer.gif_query).filter(|_| media_id.is_none());
    Ok(ReplyDraft {
        text: truncate_at_boundary(&reply, MAX_REPLY_LENGTH),
        gif_query,
        media_id,
        incoming_tone: tone,
        thinking: answer.thinking.unwrap_or_default().trim().to_owned(),
        declined: answer
            .declined
            .iter()
            .map(|line| line.trim().to_owned())
            .filter(|line| !line.is_empty())
            .collect(),
        tools: tool_steps(calls),
        recalled: Vec::new(),
    })
}

#[cfg(test)]
#[path = "draft_tests.rs"]
mod tests;
