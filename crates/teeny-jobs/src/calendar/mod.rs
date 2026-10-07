//! The content calendar: each `calendar.json` item run on its own cron by
//! [`job::CalendarJob`], and the cron written so either weekday numbering
//! reads it the same ([`to_named_weekdays`]).

pub mod cron;
pub mod job;

pub use cron::to_named_weekdays;
pub use job::{CalendarJob, ContentDrafter, TelegramSender, dry_run, run};
