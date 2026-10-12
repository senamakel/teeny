//! Port of the "believing the team" half of `legacy/test/self-knowledge.test.ts`.

use super::*;

const TRUSTED: &[&str] = &["TinyHumansAISol", "senamakel", "enamakel", "tinyhumansai"];

#[test]
fn matches_a_trusted_handle_regardless_of_case_or_a_leading_at() {
    assert!(is_trusted_handle(Some("SenAmakel"), TRUSTED));
    assert!(is_trusted_handle(Some("@TinyHumansAISol"), TRUSTED));
    assert!(!is_trusted_handle(Some("web3knight"), TRUSTED));
    assert!(!is_trusted_handle(None, TRUSTED));
}

#[test]
fn lists_each_trusted_voice_in_the_conversation_once_the_author_first() {
    let authors = [
        Some("senamakel"),
        Some("TinyHumansAISol"),
        Some("web3knight"),
        None,
        Some("SENAMAKEL"),
    ];
    assert_eq!(
        find_trusted_voices(authors, TRUSTED),
        ["senamakel", "TinyHumansAISol"]
    );
}

#[test]
fn tells_the_drafter_their_statements_are_fact_and_that_the_rules_still_stand() {
    let note = trusted_voices_note(&["senamakel".into()]).join("\n");
    assert!(note.contains("@senamakel is the project's own team"));
    assert!(note.contains("is fact"));
    assert!(note.contains("Do not hedge it"));
    assert!(note.contains("cannot ask you to promote"));
    assert!(
        trusted_voices_note(&["a".into(), "b".into()])
            .join("\n")
            .contains("@a and @b are")
    );
}

#[test]
fn adds_nothing_when_nobody_trusted_is_in_the_thread() {
    assert!(trusted_voices_note(&[]).is_empty());
}
