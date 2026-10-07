//! Loading `calendar.json` for scheduling: teeny-cadence's checks, then
//! every item's weekday written as names ([`named_weekdays`]). An item
//! whose weekday cannot be named refuses the whole calendar, by item, at
//! load: the service does not start with a cron the scheduler would misread.

use std::path::Path;

use teeny_cadence::calendar::{Calendar, CalendarError};

use super::cron::{WeekdayError, named_weekdays};

/// Why a calendar was not loaded.
#[derive(Debug, thiserror::Error)]
pub enum CalendarLoadError {
    #[error(transparent)]
    Calendar(#[from] CalendarError),
    #[error("calendar: items[{index}].cron: {error}")]
    Weekday { index: usize, error: WeekdayError },
}

/// Reads `CALENDAR_PATH`, with every item's cron in named weekdays.
pub fn load_calendar(path: impl AsRef<Path>) -> Result<Calendar, CalendarLoadError> {
    name_weekdays(teeny_cadence::calendar::load_calendar(path)?)
}

/// `calendar` with every item's weekday written as names.
pub fn name_weekdays(mut calendar: Calendar) -> Result<Calendar, CalendarLoadError> {
    for (index, item) in calendar.items.iter_mut().enumerate() {
        item.cron = named_weekdays(&item.cron)
            .map_err(|error| CalendarLoadError::Weekday { index, error })?;
    }
    Ok(calendar)
}

#[cfg(test)]
#[path = "load_tests.rs"]
mod tests;
