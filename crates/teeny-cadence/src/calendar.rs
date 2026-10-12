//! The content calendar: `calendar.json`, a list of scheduled posts read at
//! startup.
//!
//! A file rather than configuration because a calendar is a list and `.env`
//! is not; JSON because a cron expression and a channel list want a schema.
//! Names must be unique: two items called `gm` would share a journal id and a
//! memory key, and the second would look like a rerun of the first.
//! A port of `legacy/src/calendar/calendar.ts`. Two departures: a cron is
//! checked against the scheduler's parser (5 fields get a leading seconds
//! field, as OpenHuman's cron does) rather than node-cron's, and the
//! timezone must be a real IANA zone, refused at load rather than at the
//! first schedule.

use std::path::Path;
use std::str::FromStr;

use chrono_tz::Tz;
use serde::Deserialize;

/// The calendar item the morning job stands in for: the service skips it
/// while the morning job runs, so renaming it would bring back a second
/// good morning.
pub const MORNING_CALENDAR_SLOT: &str = "gm";

/// Where an item is posted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    X,
    Telegram,
}

/// What the post carries. A promise as much as a permission: `Image` means
/// the post carries one, and the job refuses to publish text alone when the
/// picture did not happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Media {
    #[default]
    None,
    Image,
    Video,
}

/// The picture's or clip's shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Aspect {
    #[default]
    Square,
    Landscape,
    Portrait,
}

fn yes() -> bool {
    true
}

/// One scheduled post. `brief` is what the account is asked to write about,
/// in the operator's words; the model turns it into the post in its voice.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarItem {
    /// Stable key: journal ids, memory, and the "already ran today" guard.
    pub name: String,
    pub cron: String,
    pub channels: Vec<Channel>,
    pub brief: String,
    #[serde(default)]
    pub media: Media,
    #[serde(default)]
    pub aspect: Aspect,
    /// Whether Tiny is in the picture. Almost always.
    #[serde(default = "yes")]
    pub features_tiny: bool,
    /// Telegram chats to post to; falls back to `CALENDAR_TELEGRAM_CHAT_IDS`.
    #[serde(default)]
    pub telegram_chat_ids: Option<Vec<String>>,
    #[serde(default = "yes")]
    pub enabled: bool,
}

/// The whole calendar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Calendar {
    /// The zone every cron is read in; `None` means the service's.
    pub timezone: Option<Tz>,
    pub items: Vec<CalendarItem>,
}

/// A calendar that would misfire, and why.
#[derive(Debug, thiserror::Error)]
#[error("calendar: {0}")]
pub struct CalendarError(String);

#[derive(Deserialize)]
struct RawCalendar {
    #[serde(default)]
    timezone: Option<String>,
    items: Vec<CalendarItem>,
}

/// Whether the scheduler can read this cron: five fields (a leading seconds
/// field is added), or six or seven in the crate's own syntax.
pub fn is_cron_expression(expression: &str) -> bool {
    let expression = expression.trim();
    let normalized = match expression.split_whitespace().count() {
        5 => format!("0 {expression}"),
        6 | 7 => expression.to_owned(),
        _ => return false,
    };
    cron::Schedule::from_str(&normalized).is_ok()
}

fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && name.len() <= 40
}

fn valid_chat_id(id: &str) -> bool {
    let digits = id.strip_prefix('-').unwrap_or(id);
    !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())
}

fn check(index: usize, item: &CalendarItem) -> Result<(), CalendarError> {
    let refuse =
        |field: &str, why: &str| Err(CalendarError(format!("items[{index}].{field}: {why}")));
    if !valid_name(&item.name) {
        return refuse("name", "lowercase letters, digits and dashes");
    }
    if !is_cron_expression(&item.cron) {
        return refuse("cron", "not a cron expression");
    }
    if item.channels.is_empty() {
        return refuse("channels", "at least one channel");
    }
    let brief = item.brief.encode_utf16().count();
    if !(10..=4_000).contains(&brief) {
        return refuse("brief", "between 10 and 4000 characters");
    }
    if item
        .telegram_chat_ids
        .iter()
        .flatten()
        .any(|id| !valid_chat_id(id))
    {
        return refuse("telegramChatIds", "chat ids are numbers");
    }
    Ok(())
}

/// Reads a calendar and refuses anything that would misfire.
pub fn parse_calendar(text: &str) -> Result<Calendar, CalendarError> {
    let raw: RawCalendar =
        serde_json::from_str(text).map_err(|error| CalendarError(error.to_string()))?;
    let timezone =
        match raw.timezone {
            None => None,
            Some(zone) => Some(zone.parse::<Tz>().map_err(|_| {
                CalendarError(format!("timezone: \"{zone}\" is not an IANA timezone"))
            })?),
        };
    if raw.items.is_empty() {
        return Err(CalendarError("at least one item".into()));
    }
    let mut names: Vec<&str> = Vec::new();
    for (index, item) in raw.items.iter().enumerate() {
        check(index, item)?;
        if names.contains(&item.name.as_str()) {
            return Err(CalendarError(format!(
                "two items are named \"{}\"",
                item.name
            )));
        }
        names.push(&item.name);
    }
    Ok(Calendar {
        timezone,
        items: raw.items,
    })
}

/// Reads `CALENDAR_PATH`.
pub fn load_calendar(path: impl AsRef<Path>) -> Result<Calendar, CalendarError> {
    let path = path.as_ref();
    let text = std::fs::read_to_string(path)
        .map_err(|error| CalendarError(format!("{}: {error}", path.display())))?;
    parse_calendar(&text)
}

#[cfg(test)]
#[path = "calendar_tests.rs"]
mod tests;
