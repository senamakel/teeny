//! Port of `legacy/test/calendar.test.ts`.

use serde_json::{Value, json};

use super::*;

fn gm() -> Value {
    json!({
        "name": "gm",
        "cron": "0 7 * * *",
        "channels": ["x", "telegram"],
        "media": "image",
        "brief": "A good-morning post, one line, Tiny with the morning.",
    })
}

fn with(overrides: Value) -> Value {
    let mut item = gm();
    for (key, value) in overrides.as_object().unwrap() {
        item[key] = value.clone();
    }
    item
}

fn parse(items: Vec<Value>) -> Result<Calendar, CalendarError> {
    parse_calendar(&json!({ "items": items }).to_string())
}

fn shipped() -> Calendar {
    load_calendar(concat!(env!("CARGO_MANIFEST_DIR"), "/../../calendar.json")).unwrap()
}

#[test]
fn parses_an_item_and_fills_in_the_defaults() {
    let calendar = parse(vec![gm()]).unwrap();
    let item = &calendar.items[0];
    assert_eq!(item.name, "gm");
    assert_eq!(item.media, Media::Image);
    assert_eq!(item.aspect, Aspect::Square);
    assert!(item.features_tiny);
    assert!(item.enabled);
    assert_eq!(item.channels, [Channel::X, Channel::Telegram]);
    assert_eq!(calendar.timezone, None);
}

#[test]
fn refuses_what_would_misfire_a_bad_cron_a_bad_name_two_items_with_one_name() {
    let message = |result: Result<Calendar, CalendarError>| result.unwrap_err().to_string();
    assert!(message(parse(vec![with(json!({ "cron": "every morning" }))])).contains("cron"));
    assert!(message(parse(vec![with(json!({ "name": "GM post" }))])).contains("lowercase"));
    assert!(
        message(parse(vec![
            gm(),
            with(json!({ "brief": "another one entirely" }))
        ]))
        .contains("two items are named \"gm\"")
    );
    assert!(parse(vec![]).is_err());
    assert!(parse(vec![with(json!({ "channels": [] }))]).is_err());
    assert!(parse(vec![with(json!({ "telegramChatIds": ["holders"] }))]).is_err());
}

#[test]
fn refuses_a_brief_too_short_to_write_from_and_an_unknown_timezone() {
    assert!(parse(vec![with(json!({ "brief": "gm" }))]).is_err());
    let zoned = |tz: &str| parse_calendar(&json!({ "timezone": tz, "items": [gm()] }).to_string());
    assert_eq!(
        zoned("Europe/Berlin").unwrap().timezone,
        Some(chrono_tz::Europe::Berlin)
    );
    assert!(
        zoned("Mars/Olympus")
            .unwrap_err()
            .to_string()
            .contains("timezone")
    );
}

#[test]
fn accepts_the_cron_shapes_the_scheduler_reads() {
    for good in [
        "0 7 * * *",
        "*/5 * * * *",
        "0 16 * * 5",
        "0 7 * * MON-FRI",
        "0 0 7 * * *",
    ] {
        assert!(is_cron_expression(good), "{good}");
    }
    for bad in [
        "every morning",
        "0 7 * *",
        "60 7 * * *",
        "0 24 * * *",
        "0 7 32 * *",
    ] {
        assert!(!is_cron_expression(bad), "{bad}");
    }
}

#[test]
fn ships_with_a_calendar_that_loads() {
    let calendar = shipped();
    assert!(calendar.items.iter().any(|item| item.name == "gm"));
    assert!(calendar.items.iter().all(|item| item.brief.len() > 20));
}

#[test]
fn names_the_good_morning_item_the_morning_job_stands_in_for() {
    // The service skips this item while the morning job runs; renaming it in
    // calendar.json would quietly bring back the second good morning.
    let calendar = shipped();
    let slot = calendar
        .items
        .iter()
        .find(|item| item.name == MORNING_CALENDAR_SLOT)
        .unwrap();
    assert!(slot.brief.contains("good-morning"));
}
