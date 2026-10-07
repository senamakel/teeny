//! Calendar crons that mean the same under either weekday numbering.
//!
//! A calendar item's cron is POSIX: weekday 0 (or 7) is Sunday. The `cron`
//! crate that OpenHuman's scheduler hands it to counts 1 as Sunday, so `5`
//! fires on Thursday there (a fix is pending upstream, tinyflows#106).
//! Names are read the same way by both, so numeric weekdays are written as
//! names before an item is scheduled: `5` is `FRI`, `1-5` is `MON-FRI`, and a
//! step (`*/2`, `1-5/2`), whose meaning depends on where counting starts, is
//! spelled out as the list of days POSIX means.

const DAYS: [&str; 7] = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];

fn day(value: &str) -> Option<u32> {
    value.parse::<u32>().ok().filter(|day| *day <= 7)
}

fn name(day: u32) -> &'static str {
    DAYS[(day % 7) as usize]
}

/// Every day from `from` to `to` (POSIX numbers) in steps of `step`, named.
fn spelled(from: u32, to: u32, step: u32) -> Vec<&'static str> {
    (from..=to)
        .step_by(step.max(1) as usize)
        .map(name)
        .collect()
}

/// One comma-separated part of the weekday field, named; `None` when it is
/// not something this can read (left as written).
fn part(part: &str) -> Option<Vec<String>> {
    let owned = |names: Vec<&'static str>| names.into_iter().map(str::to_owned).collect();
    let (range, step) = match part.split_once('/') {
        Some((range, step)) => (range, Some(step.parse::<u32>().ok()?)),
        None => (part, None),
    };
    if part.chars().all(|c| c.is_ascii_alphabetic() || c == '-') {
        return Some(vec![part.to_uppercase()]);
    }
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
    match step {
        Some(step) => Some(owned(spelled(from, to, step))),
        None if from == to => Some(vec![name(from).to_owned()]),
        // A range ending on 7, or wrapping, is spelled out day by day.
        None if to == 7 || from > to => Some(owned(if from > to {
            spelled(from, 6, 1)
                .into_iter()
                .chain(spelled(0, to, 1))
                .collect()
        } else {
            spelled(from, to, 1)
        })),
        None => Some(vec![format!("{}-{}", name(from), name(to))]),
    }
}

/// `expression` with its weekday field named. Five fields (POSIX) or six
/// and seven (a seconds field first); anything else is returned as it is,
/// as is a weekday field that is `*`, `?` or not readable.
pub fn to_named_weekdays(expression: &str) -> String {
    let mut fields: Vec<String> = expression.split_whitespace().map(str::to_owned).collect();
    let index = match fields.len() {
        5 => 4,
        6 | 7 => 5,
        _ => return expression.to_owned(),
    };
    let weekday = &fields[index];
    if weekday == "*" || weekday == "?" {
        return fields.join(" ");
    }
    let mut named: Vec<String> = Vec::new();
    for piece in weekday.split(',') {
        match part(piece) {
            Some(names) => named.extend(names),
            None => return fields.join(" "),
        }
    }
    named.dedup();
    fields[index] = named.join(",");
    fields.join(" ")
}

#[cfg(test)]
#[path = "cron_tests.rs"]
mod tests;
