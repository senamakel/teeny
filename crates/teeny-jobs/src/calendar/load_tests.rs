use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

/// A calendar file, removed when dropped.
struct File(PathBuf);

impl File {
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for File {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn file(json: &str) -> File {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "teeny-calendar-{}-{}.json",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::write(&path, json).unwrap();
    File(path)
}

fn calendar(crons: &[&str]) -> String {
    let items: Vec<String> = crons
        .iter()
        .enumerate()
        .map(|(index, cron)| {
            format!(
                r#"{{"name": "item-{index}", "cron": "{cron}", "channels": ["x"], "brief": "say good evening to the holders"}}"#
            )
        })
        .collect();
    format!(r#"{{"items": [{}]}}"#, items.join(","))
}

#[test]
fn loads_a_calendar_with_every_weekday_written_as_names() {
    let path = file(&calendar(&[
        "0 9 * * 1-5",
        "0 18 * * MON-FRI/2",
        "30 7 * * *",
    ]));
    let loaded = load_calendar(path.path()).unwrap();
    let crons: Vec<&str> = loaded.items.iter().map(|item| item.cron.as_str()).collect();
    assert_eq!(
        crons,
        ["0 9 * * MON-FRI", "0 18 * * MON,WED,FRI", "30 7 * * *"]
    );
}

#[test]
fn refuses_an_item_whose_weekday_cannot_be_named_saying_which() {
    let mut parsed =
        teeny_cadence::calendar::parse_calendar(&calendar(&["0 9 * * 1", "0 9 * * 2"])).unwrap();
    // A form the calendar's own parser would let through, were it to.
    parsed.items[1].cron = "0 9 * * 5#2".into();
    let error = name_weekdays(parsed).unwrap_err().to_string();
    assert!(error.contains("items[1].cron"), "{error}");
    assert!(error.contains("5#2"), "{error}");
}

#[test]
fn refuses_at_load_what_the_scheduler_would_misread() {
    for cron in ["0 9 * * 5#2", "0 9 * * L"] {
        let path = file(&calendar(&[cron]));
        let error = load_calendar(path.path()).unwrap_err().to_string();
        assert!(error.contains("items[0].cron"), "{cron}: {error}");
    }
}
