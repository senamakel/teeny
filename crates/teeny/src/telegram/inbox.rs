//! What comes in from Telegram, read the way `legacy/src/telegram/inbox.ts`
//! read it: whether a line is for the account, whether a person wrote it,
//! and the line as a [`ChatMessage`] without the account's handle in it.
//! Plus `splitForTelegram` from `legacy/src/clients/telegram.ts`.

use chrono::DateTime;
use regex::{Regex, RegexBuilder};
use serde::Deserialize;
use serde_json::{Map, Value};
use teeny_cadence::clock::iso;
use teeny_jobs::chat::ChatMessage;
use teeny_memory::ChatKind;

/// Telegram's longest text message.
pub const MAX_MESSAGE_LENGTH: usize = 4_096;

/// The bot, as `getMe` names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BotIdentity {
    pub id: i64,
    pub username: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Update {
    pub update_id: i64,
    #[serde(default)]
    pub message: Option<Message>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Chat {
    pub id: i64,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct User {
    pub id: i64,
    #[serde(default)]
    pub is_bot: bool,
    #[serde(default)]
    pub first_name: String,
    #[serde(default)]
    pub last_name: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Entity {
    #[serde(rename = "type")]
    pub kind: String,
    /// In UTF-16 code units, as Telegram counts.
    pub offset: usize,
    pub length: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Document {
    #[serde(default)]
    pub file_id: Option<String>,
    #[serde(default)]
    pub mime_type: Option<String>,
}

/// One size of a photo.
#[derive(Debug, Clone, Deserialize)]
struct PhotoSize {
    file_id: String,
    #[serde(default)]
    width: u64,
    #[serde(default)]
    height: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Message {
    pub message_id: i64,
    pub date: i64,
    pub chat: Chat,
    #[serde(default)]
    pub from: Option<User>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub caption: Option<String>,
    #[serde(default)]
    pub entities: Option<Vec<Entity>>,
    #[serde(default)]
    pub caption_entities: Option<Vec<Entity>>,
    #[serde(default)]
    pub reply_to_message: Option<Box<Message>>,
    #[serde(default)]
    pub message_thread_id: Option<i64>,
    #[serde(default)]
    pub sender_chat: Option<Value>,
    #[serde(default)]
    pub is_automatic_forward: Option<bool>,
    #[serde(default)]
    pub photo: Option<Vec<Value>>,
    #[serde(default)]
    pub document: Option<Document>,
    /// Everything else, for the service-message check.
    #[serde(flatten)]
    pub rest: Map<String, Value>,
}

/// Fields only Telegram's own messages (a join, a pin, a title) carry.
const SERVICE_FIELDS: [&str; 30] = [
    "new_chat_members",
    "left_chat_member",
    "new_chat_title",
    "new_chat_photo",
    "delete_chat_photo",
    "group_chat_created",
    "supergroup_chat_created",
    "channel_chat_created",
    "pinned_message",
    "migrate_to_chat_id",
    "migrate_from_chat_id",
    "message_auto_delete_timer_changed",
    "video_chat_started",
    "video_chat_ended",
    "video_chat_scheduled",
    "video_chat_participants_invited",
    "forum_topic_created",
    "forum_topic_edited",
    "forum_topic_closed",
    "forum_topic_reopened",
    "general_forum_topic_hidden",
    "general_forum_topic_unhidden",
    "boost_added",
    "giveaway_created",
    "successful_payment",
    "users_shared",
    "chat_shared",
    "write_access_allowed",
    "proximity_alert_triggered",
    "web_app_data",
];

const IMAGE_TYPES: [&str; 4] = ["image/png", "image/jpeg", "image/webp", "image/gif"];

fn words(message: &Message) -> &str {
    message
        .text
        .as_deref()
        .or(message.caption.as_deref())
        .unwrap_or_default()
}

fn pattern(source: &str) -> Regex {
    RegexBuilder::new(source)
        .case_insensitive(true)
        .build()
        .expect("a handle is escaped into a valid pattern")
}

/// Whether a line is for the account. In a DM everything is; in a group
/// only a mention of its handle, a reply to one of its lines, or a command
/// addressed to it, so an admin turning privacy mode off does not make the
/// account answer every line.
pub fn is_addressed(message: &Message, bot: &BotIdentity) -> bool {
    if message.chat.kind == "private" {
        return true;
    }
    let replies_to_bot = message
        .reply_to_message
        .as_ref()
        .and_then(|parent| parent.from.as_ref())
        .is_some_and(|author| author.id == bot.id);
    if replies_to_bot {
        return true;
    }
    let text = words(message);
    let units: Vec<u16> = text.encode_utf16().collect();
    let handle = format!("@{}", bot.username.to_lowercase());
    let entities = message
        .entities
        .iter()
        .chain(message.caption_entities.iter())
        .flatten();
    for entity in entities.filter(|entity| entity.kind == "mention") {
        let end = (entity.offset + entity.length).min(units.len());
        let start = entity.offset.min(end);
        if String::from_utf16_lossy(&units[start..end]).to_lowercase() == handle {
            return true;
        }
    }
    pattern(&format!(r"^/\w+@{}\b", regex::escape(&bot.username))).is_match(text)
}

/// The text with the account's handle taken out, so the drafter reads the
/// question rather than the mention.
pub fn strip_mention(text: &str, bot: &BotIdentity) -> String {
    let handle = regex::escape(&bot.username);
    let command = pattern(&format!(r"^(/\w+)@{handle}\b"));
    let text = command.replace(text, "$1");
    let text = pattern(&format!(r"@{handle}\b")).replace_all(&text, "");
    Regex::new(r"[ \t]{2,}")
        .expect("valid")
        .replace_all(&text, " ")
        .trim()
        .to_owned()
}

/// The command a message starts with, without the bot suffix, lowercased.
pub fn command_of(message: &Message) -> Option<String> {
    let text = message.text.as_deref()?;
    pattern(r"^/([a-z0-9_]+)(?:@\w+)?(?:\s|$)")
        .captures(text)
        .map(|captures| captures[1].to_lowercase())
}

/// Software rather than a person: a bot, a channel or anonymous admin, or a
/// channel post copied into its discussion group.
pub fn is_from_bot(message: &Message) -> bool {
    match &message.from {
        None => true,
        Some(author) => {
            author.is_bot
                || message.sender_chat.is_some()
                || message.is_automatic_forward == Some(true)
        }
    }
}

/// Telegram's own line: a join, a leave, a pin, a title change.
pub fn is_service(message: &Message) -> bool {
    SERVICE_FIELDS
        .iter()
        .any(|field| message.rest.contains_key(*field))
}

/// How many pictures the message carries: its photo, or an image sent as a
/// document.
fn pictures(message: &Message) -> usize {
    let photo = message
        .photo
        .as_ref()
        .is_some_and(|sizes| !sizes.is_empty());
    let document = message
        .document
        .as_ref()
        .and_then(|document| document.mime_type.as_deref())
        .is_some_and(|mime| IMAGE_TYPES.contains(&mime));
    usize::from(photo) + usize::from(document)
}

/// At most this many pictures of one message are written in, or looked at.
pub const MAX_PICTURES: usize = 4;

/// The Telegram file ids of the message's pictures, in the order
/// [`to_chat_message`] writes them in: the photo at its largest size, then
/// an image sent as a document. Only ever ids for Telegram's own file API:
/// nothing in a message is fetched as a URL.
pub fn picture_files(message: &Message) -> Vec<String> {
    let largest = message
        .photo
        .iter()
        .flatten()
        .filter_map(|size| serde_json::from_value::<PhotoSize>(size.clone()).ok())
        .enumerate()
        // The largest by area; Telegram lists them smallest first, so the
        // later one wins a tie.
        .max_by_key(|(index, size)| (size.width * size.height, *index))
        .map(|(_, size)| size.file_id);
    let document = message
        .document
        .as_ref()
        .filter(|document| {
            document
                .mime_type
                .as_deref()
                .is_some_and(|mime| IMAGE_TYPES.contains(&mime))
        })
        .and_then(|document| document.file_id.clone());
    largest
        .into_iter()
        .chain(document)
        .take(MAX_PICTURES)
        .collect()
}

/// A person's line as the chat turn reads it, or `None` when it is not a
/// person saying something: no text and no picture, a service message, a
/// bot or a channel, or the account itself. A picture is written in as
/// `[sent a photo]`; [`with_pictures_seen`] writes in what was in it.
pub fn to_chat_message(message: &Message, bot: &BotIdentity) -> Option<ChatMessage> {
    let unseen = vec!["[sent a photo]".to_owned(); pictures(message).min(MAX_PICTURES)];
    with_pictures_seen(message, bot, unseen)
}

/// [`to_chat_message`] with `notes` (one per picture, `[image: …]`) in
/// place of the pictures.
pub fn with_pictures_seen(
    message: &Message,
    bot: &BotIdentity,
    notes: Vec<String>,
) -> Option<ChatMessage> {
    let author = message.from.as_ref()?;
    if is_from_bot(message) || is_service(message) || author.id == bot.id {
        return None;
    }
    let text = strip_mention(words(message), bot);
    if text.is_empty() && notes.is_empty() {
        return None;
    }
    let text = std::iter::once(text)
        .chain(notes)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let name = std::iter::once(author.first_name.as_str())
        .chain(author.last_name.as_deref())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    Some(ChatMessage {
        id: message.message_id.to_string(),
        chat_id: message.chat.id.to_string(),
        chat_kind: if message.chat.kind == "private" {
            ChatKind::Private
        } else {
            ChatKind::Group
        },
        chat_title: message.chat.title.clone(),
        text,
        author_id: author.id.to_string(),
        author_username: author.username.clone(),
        author_name: (!name.is_empty()).then_some(name),
        created_at: DateTime::from_timestamp(message.date, 0).map(iso),
        message_thread_id: message.message_thread_id.map(|id| id.to_string()),
        batch_ids: Vec::new(),
    })
}

/// `text` in pieces of at most `limit` characters, cut at a paragraph, then
/// a line, then a word, when one falls past the first quarter of the piece.
pub fn split_for_telegram(text: &str, limit: usize) -> Vec<String> {
    let trimmed = text.trim();
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() <= limit {
        return if trimmed.is_empty() {
            Vec::new()
        } else {
            vec![trimmed.to_owned()]
        };
    }
    let mut pieces = Vec::new();
    let mut remaining = chars;
    while remaining.len() > limit {
        let window = &remaining[..limit];
        let last = |needle: &[char]| {
            window
                .windows(needle.len())
                .rposition(|candidate| candidate == needle)
        };
        let cut = [last(&['\n', '\n']), last(&['\n']), last(&[' '])]
            .into_iter()
            .flatten()
            .find(|index| *index > limit / 4)
            .unwrap_or(limit);
        let piece: String = remaining[..cut].iter().collect();
        pieces.push(piece.trim().to_owned());
        let rest: String = remaining[cut..].iter().collect();
        remaining = rest.trim().chars().collect();
    }
    if !remaining.is_empty() {
        pieces.push(remaining.into_iter().collect());
    }
    pieces
}

#[cfg(test)]
#[path = "inbox_tests.rs"]
mod tests;
