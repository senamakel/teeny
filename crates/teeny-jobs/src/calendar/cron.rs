//! Calendar crons that mean the same under either weekday numbering.
//!
//! A calendar item's cron is POSIX: weekday 0 (or 7) is Sunday. The `cron`
//! crate that OpenHuman's scheduler hands it to counts 1 as Sunday, so `5`
//! fires on Thursday there (a fix is pending upstream, tinyflows#106).
//! Names are read the same way by both, so every weekday is written as names
//! before an item is scheduled: `5` is `FRI`, `1-5` is `MON-FRI`, and a step
//! (`*/2`, `1-5/2`, `MON-FRI/2`), whose meaning depends on where counting
//! starts, is spelled out as the list of days POSIX means. A weekday that
//! cannot be written with names (`5#2`, `L`, `5L`, `1W`, an unknown name) is
//! refused ([`named_weekdays`]), and the calendar is refused with it at load
//! ([`super::load`]): a numeric weekday never reaches the scheduler.

const DAYS: [&str; 7] = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];

/// A weekday field that cannot be written with day names.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "the weekday {field:?} cannot be written with day names, and the scheduler reads a numeric weekday differently (1 is Sunday there). Use day names or numbers 0-7, with ranges, lists and steps; not #, L or W"
)]
pub struct WeekdayError {
    /// The weekday field as written.
    pub field: String,
}

/// A day, by POSIX number (0 or 7 is Sunday) or by three-letter name.
fn day(value: &str) -> Option<u32> {
    if let Ok(number) = value.parse::<u32>() {
        return (number <= 7).then_some(number);
    }
    DAYS.iter()
        .position(|name| name.eq_ignore_ascii_case(value))
        .map(|index| index as u32)
}

fn name(day: u32) -> &'static str {
    DAYS[(day % 7) as usize]
}

/// Every day from `from` to `to` (POSIX numbers) in steps of `step`, named.
fn spelled(from: u32, to: u32, step: u32) -> Vec<String> {
    (from..=to)
        .step_by(step as usize)
        .map(|day| name(day).to_owned())
        .collect()
}

/// One comma-separated part of the weekday field, named; `None` when it is
/// not something names can say.
fn part(part: &str) -> Option<Vec<String>> {
    let (range, step) = match part.split_once('/') {
        Some((range, step)) => (range, Some(step.parse::<u32>().ok().filter(|s| *s > 0)?)),
        None => (part, None),
    };
    let (from, to) = match range {
        "*" => (0, 6),
        _ => match range.split_once('-') {
            Some((from, to)) => (day(from)?, day(to)?),
            None => {
                let from = day(range)?;
                (from, if step.is_some() { 6 } else { from })
            }
        },
    };
    Some(match step {
        Some(step) if from <= to => spelled(from, to, step),
        // A stepped range that wraps is not something cron can say either.
        Some(_) => return None,
        None if from == to => vec![name(from).to_owned()],
        // A range that wraps (`SAT-SUN`, `6-1`) is spelled out day by day.
        None if from > to => spelled(from, 6, 1)
            .into_iter()
            .chain(spelled(0, to, 1))
            .collect(),
        // A range ending on 7 is Sunday-inclusive: `SUN-SAT` when it is the
        // whole week, else spelled out.
        None if to == 7 && from <= 1 => vec!["SUN-SAT".to_owned()],
        None if to == 7 => spelled(from, 7, 1),
        None => vec![format!("{}-{}", name(from), name(to))],
    })
}

/// `expression` with its weekday field written as names. Five fields
/// (POSIX) or six and seven (a seconds field first); anything else is
/// returned as it is, having no weekday to misread, as is a weekday field of
/// `*` or `?`.
pub fn named_weekdays(expression: &str) -> Result<String, WeekdayError> {
    let mut fields: Vec<String> = expression.split_whitespace().map(str::to_owned).collect();
    let index = match fields.len() {
        5 => 4,
        6 | 7 => 5,
        _ => return Ok(expression.to_owned()),
    };
    let weekday = &fields[index];
    if weekday == "*" || weekday == "?" {
        return Ok(fields.join(" "));
    }
    let refused = || WeekdayError {
        field: weekday.clone(),
    };
    let mut named: Vec<String> = Vec::new();
    for piece in weekday.split(',') {
        for day in part(piece).ok_or_else(refused)? {
            if !named.contains(&day) {
                named.push(day);
            }
        }
    }
    fields[index] = named.join(",");
    Ok(fields.join(" "))
}

/// [`named_weekdays`], for a cron already through the calendar's load,
/// which refuses any it cannot name: such a cron is returned as written.
pub fn to_named_weekdays(expression: &str) -> String {
    named_weekdays(expression).unwrap_or_else(|_| expression.to_owned())
}

#[cfg(test)]
#[path = "cron_tests.rs"]
mod tests;
