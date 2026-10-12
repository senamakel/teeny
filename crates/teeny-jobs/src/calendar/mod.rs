//! The content calendar: each `calendar.json` item run on its own cron by
//! [`job::CalendarJob`], loaded ([`load_calendar`]) with the cron written so
//! either weekday numbering reads it the same, or refused.

pub mod cron;
pub mod job;
pub mod load;

pub use cron::{WeekdayError, named_weekdays, to_named_weekdays};
pub use job::{CalendarJob, ContentDrafter, TelegramSender, dry_run, run};
pub use load::{CalendarLoadError, load_calendar};
