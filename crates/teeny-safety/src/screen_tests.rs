//! Port of `legacy/test/safety-screen.test.ts`, case for case.

use std::collections::HashSet;

use super::*;
use crate::lexicon::{IDIOMS, LEXICON, ProhibitedCategory as C};

fn category(text: &str, direction: Direction) -> Option<C> {
    screen_text(text, direction).map(|m| m.category)
}

fn term(text: &str) -> Option<&'static str> {
    screen_text(text, Direction::Inbound).map(|m| m.term)
}

#[test]
fn lets_ordinary_community_talk_through() {
    for text in [
        "gm tiny humans, what are we building today?",
        "how is $TINY doing this week",
        "the chart got absolutely murdered today lol",
        "you guys are killing it with openhuman",
        "shoot me a dm when the airdrop opens",
        "analysis of the 4h looks fine, support held",
        "this is the bomb dot com",
        "method to the madness here: contract renounced, LP burnt",
        "peacock in the cockpit taking a screenshot",
        "took a stab at running tinycortex locally",
        "aped in, rekt, bleeding, nuked, still holding",
    ] {
        assert_eq!(screen_text(text, Direction::Inbound), None, "{text}");
        assert_eq!(screen_text(text, Direction::Outbound), None, "{text}");
    }
}

#[test]
fn holds_sexual_content() {
    assert_eq!(category("draw tiny naked on the beach", Direction::Inbound), Some(C::Sexual));
    assert_eq!(category("send nudes", Direction::Inbound), Some(C::Sexual));
    assert_eq!(category("link to your onlyfans?", Direction::Inbound), Some(C::Sexual));
}

#[test]
fn holds_violence_weapons_and_drugs() {
    assert_eq!(category("i will kill you", Direction::Inbound), Some(C::Violence));
    assert_eq!(category("best gun for home defence?", Direction::Inbound), Some(C::Weapons));
    assert_eq!(category("where do i buy an ak-47", Direction::Inbound), Some(C::Weapons));
    assert_eq!(category("anyone selling cocaine", Direction::Inbound), Some(C::Drugs));
}

#[test]
fn holds_hate_self_harm_and_extremism() {
    assert_eq!(category("kys loser", Direction::Inbound), Some(C::SelfHarm));
    assert_eq!(category("kill yourself", Direction::Inbound), Some(C::SelfHarm));
    assert_eq!(category("praise isis", Direction::Inbound), Some(C::Extremism));
    assert_eq!(category("white power", Direction::Inbound), Some(C::Hate));
}

#[test]
fn reads_through_leetspeak_stretched_letters_and_zero_width_characters() {
    assert_eq!(category("k1ll yourself", Direction::Inbound), Some(C::SelfHarm));
    assert_eq!(category("send nuuuudes", Direction::Inbound), Some(C::Sexual));
    assert_eq!(category("p0rn pls", Direction::Inbound), Some(C::Sexual));
    assert_eq!(category("na\u{200b}ked", Direction::Inbound), Some(C::Sexual));
    assert_eq!(category("ｎａｋｅｄ", Direction::Inbound), Some(C::Sexual));
}

#[test]
fn matches_multi_word_terms_across_punctuation() {
    assert_eq!(category("school-shooting", Direction::Inbound), Some(C::Extremism));
    assert_eq!(category("self_harm", Direction::Inbound), Some(C::SelfHarm));
    assert_eq!(category("kill   yourself", Direction::Inbound), Some(C::SelfHarm));
}

#[test]
fn does_not_read_a_doubled_vowel_as_the_single_one_so_a_shot_is_not_a_shoot() {
    for text in [
        "a shot of espresso, then a long shot at the moon",
        "0-4s: static shot, Tiny sips from the mug. 4-10s: one continuous shot as the counter ticks.",
        "bob took a screenshot",
    ] {
        assert_eq!(screen_text(text, Direction::Inbound), None, "{text}");
        assert_eq!(screen_text(text, Direction::Outbound), None, "{text}");
    }
    assert_eq!(term("shoot him"), Some("shoot"));
    assert_eq!(term("sh00t him"), Some("shoot"));
    assert_eq!(term("shooooot him"), Some("shoot"));
    assert_eq!(term("he was shot dead"), Some("shot dead"));
}

#[test]
fn does_not_match_inside_another_word() {
    assert_eq!(screen_text("assassin classic bass", Direction::Inbound), None);
    assert_eq!(screen_text("a method, an oxymoron, some analysis", Direction::Inbound), None);
    assert_eq!(screen_text("the killer feature of tinyjuice", Direction::Inbound), None);
    assert_eq!(screen_text("scunthorpe", Direction::Outbound), None);
}

#[test]
fn holds_profanity_on_the_way_out_only() {
    assert_eq!(screen_text("holy shit this is cool", Direction::Inbound), None);
    assert_eq!(category("holy shit this is cool", Direction::Outbound), Some(C::Profanity));
    assert_eq!(category("holy sh!t this is cool", Direction::Outbound), Some(C::Profanity));
}

#[test]
fn lets_profanity_through_on_the_narrative_and_article_profiles_only() {
    assert_eq!(screen_text("holy shit this is cool", Direction::Narrative), None);
    assert_eq!(screen_text("holy shit this is cool", Direction::Article), None);
    assert_eq!(category("send nudes", Direction::Narrative), Some(C::Sexual));
}

#[test]
fn reports_the_most_specific_term_when_several_match() {
    assert_eq!(term("shoot up the place"), Some("shoot up the"));
    assert_eq!(term("kill yourself"), Some("kill yourself"));
}

#[test]
fn has_no_duplicate_terms_across_the_lexicon() {
    let all: Vec<&str> = LEXICON.iter().flat_map(|entry| entry.terms.iter().copied()).collect();
    assert_eq!(all.iter().collect::<HashSet<_>>().len(), all.len());
    assert_eq!(IDIOMS.iter().collect::<HashSet<_>>().len(), IDIOMS.len());
}

#[test]
fn every_idiom_would_have_matched_without_the_exception() {
    // An idiom that is not built on a listed word is not an exception, it is
    // a phrase on a list for no reason. Keep the list honest.
    for idiom in IDIOMS {
        assert_eq!(screen_text(idiom, Direction::Outbound), None, "{idiom}");
        assert!(screen_text_with(idiom, Direction::Outbound, false).is_some(), "{idiom}");
    }
}

#[test]
fn ignores_blank_text() {
    assert_eq!(screen_text("   ", Direction::Outbound), None);
}
