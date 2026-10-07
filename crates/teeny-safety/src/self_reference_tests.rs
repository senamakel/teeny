//! Port of the "recognising itself" half of `legacy/test/self-knowledge.test.ts`.

use super::*;

const ROOT: &str = "Teeny @teenydroid just self accumulated 1.6% of solana:2AF7CqwieUjUPALL7icuZtL3X7wENdjUjGBMmfV2pump supply and became the largest holder all by itself.\n\nCan @teenydroid become the first AI @Pumpfun @fomo KOL? 😂";

fn found(text: &str) -> Vec<String> {
    find_self_references(text, "teenydroid", DEFAULT_SELF_NAMES)
}

#[test]
fn finds_its_handle_when_somebody_talks_about_it_not_just_to_it() {
    assert_eq!(found(ROOT), ["@teenydroid", "Teeny"]);
}

#[test]
fn ignores_the_leading_run_of_handles_x_carries_along_on_a_reply() {
    assert!(found("@senamakel @teenydroid wen ser").is_empty());
}

#[test]
fn still_counts_a_handle_typed_on_purpose_after_the_leading_run() {
    assert_eq!(found("@senamakel oh hi fren @teenydroid"), ["@teenydroid"]);
}

#[test]
fn matches_the_name_as_a_whole_word_only() {
    assert!(found("teenyweeny things").is_empty());
    assert_eq!(found("go teeny go"), ["teeny"]);
}

#[test]
fn does_not_mistake_a_different_handle_or_a_different_token_for_itself() {
    assert!(found("@teeny is a different account and $TEENY a different coin").is_empty());
}

#[test]
fn takes_the_names_from_configuration() {
    assert_eq!(find_self_references("Tiny is the mascot", "teenydroid", &["Tiny"]), ["Tiny"]);
    assert!(find_self_references("Teeny is the mascot", "teenydroid", &["Tiny"]).is_empty());
}

// The exact correction that was needed: it answered in the third person.
#[test]
fn tells_the_drafter_the_thread_is_about_it_and_to_answer_in_the_first_person() {
    let note = self_reference_note(&["@teenydroid".into(), "Teeny".into()]).join("\n");
    assert!(note.contains("is YOU"));
    assert!(note.contains("first person"));
    assert!(note.contains("Never"));
    assert!(note.contains("\"an AI agent\""));
}

#[test]
fn adds_nothing_when_it_is_not_being_talked_about() {
    assert!(self_reference_note(&[]).is_empty());
}
