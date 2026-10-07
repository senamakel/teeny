//! Port of `legacy/test/house-style.test.ts`, case for case, plus the two
//! question helpers the market tool and the pipelines use.

use super::*;

fn style(draft: &str) -> HouseStyleResult {
    apply_house_style(draft, &HouseStyleOptions::default())
}

fn replying(draft: &str, to: &str) -> HouseStyleResult {
    apply_house_style(draft, &HouseStyleOptions { replying_to: Some(to.into()), ..Default::default() })
}

mod count_emoji {
    use super::*;

    #[test]
    fn counts_a_sequence_once() {
        assert_eq!(count_emoji("gm 🛠️"), 1);
        assert_eq!(count_emoji("family 👨‍👩‍👧 here"), 1);
        assert_eq!(count_emoji("wave 👋🏽 once"), 1);
        assert_eq!(count_emoji("gm 🛠️ 🚀"), 2);
        assert_eq!(count_emoji("plain text"), 0);
    }
}

mod apply_house_style {
    use super::*;

    #[test]
    fn leaves_a_clean_draft_untouched() {
        let result = style("gm tiny humans. what are we building today? 🛠️");
        assert!(result.fixes.is_empty());
        assert_eq!(result.text, "gm tiny humans. what are we building today? 🛠️");
    }

    #[test]
    fn keeps_the_first_emoji_and_drops_the_rest() {
        let result = style("gm 👀 frens 💛 building 🛠️");
        assert_eq!(result.text, "gm 👀 frens building");
        assert_eq!(count_emoji(&result.text), 1);
        assert!(result.fixes.contains(&"cut 2 emoji over the cap of 1".to_owned()));
    }

    #[test]
    fn replaces_em_and_en_dashes() {
        let result = style("ready to serve something big — stay tuned");
        assert_eq!(result.text, "ready to serve something big, stay tuned");
        assert!(result.fixes.contains(&"replaced em/en dashes".to_owned()));
    }

    #[test]
    fn straightens_curly_quotes() {
        assert_eq!(style("what’re we “cooking” today").text, "what're we \"cooking\" today");
    }

    #[test]
    fn trims_an_overlong_draft() {
        let result = style(&"a".repeat(300));
        assert_eq!(result.text.len(), 280);
        assert!(result.fixes.contains(&"trimmed to 280 characters".to_owned()));
    }

    #[test]
    fn honours_a_cap_of_zero() {
        let options = HouseStyleOptions { max_emoji: 0, ..Default::default() };
        assert_eq!(apply_house_style("gm 👀", &options).text, "gm");
    }
}

mod disclaimers {
    use super::*;

    #[test]
    fn drops_a_trailing_nfa_with_the_comma_that_hung_it_on() {
        let result = style("$696K mcap today, $1 is a stop on the way not the end, nfa 🚀");
        assert_eq!(result.text, "$696K mcap today, $1 is a stop on the way not the end 🚀");
        assert!(result.fixes.contains(&"dropped a disclaimer".to_owned()));
    }

    #[test]
    fn drops_nfa_mid_sentence_in_brackets_and_in_caps() {
        assert_eq!(
            style("we are at $700K, GOAT at $400M, that is the flip (NFA) and i am not selling").text,
            "we are at $700K, GOAT at $400M, that is the flip and i am not selling"
        );
        assert_eq!(
            style("that is a 500x from here. NFA. still buying.").text,
            "that is a 500x from here. still buying."
        );
    }

    #[test]
    fn drops_the_long_forms_too() {
        assert_eq!(style("$2B is the plan, not financial advice, dyor").text, "$2B is the plan");
        assert_eq!(style("$2B is the plan - do your own research").text, "$2B is the plan");
    }

    #[test]
    fn leaves_words_that_merely_contain_the_letters_alone() {
        assert!(style("the unfair advantage of a plush toy is patience").fixes.is_empty());
    }
}

mod leading_handles {
    use super::*;

    #[test]
    fn drops_the_handle_x_adds_to_a_reply_itself() {
        let result = replying("@rafaelitewrld peekaboo 👀", "rafaelitewrld");
        assert_eq!(result.text, "peekaboo 👀");
        assert!(result.fixes.contains(&"dropped leading @handles X adds itself".to_owned()));
    }

    #[test]
    fn drops_a_leading_handle_that_is_not_the_one_being_replied_to() {
        // The observed failure: replying to HappyTinyHuman with a draft opening
        // "@senamakel", which X published as "@HappyTinyHuman @senamakel @senamakel".
        assert_eq!(replying("@senamakel me? trade? nah", "HappyTinyHuman").text, "me? trade? nah");
    }

    #[test]
    fn drops_a_whole_run_of_leading_handles() {
        assert_eq!(style("@a @b @c gm frens").text, "gm frens");
    }

    #[test]
    fn keeps_a_handle_that_is_part_of_the_sentence() {
        assert_eq!(replying("gm @someone else entirely", "rafaelitewrld").text, "gm @someone else entirely");
    }

    #[test]
    fn keeps_the_text_when_it_is_nothing_but_handles() {
        assert_eq!(style("@a @b").text, "@a @b");
    }
}

mod repeated_handles {
    use super::*;

    #[test]
    fn keeps_the_name_but_drops_the_at_when_x_already_tags_them() {
        let result = replying("heyyy @rafaelitewrld shy peek returns", "rafaelitewrld");
        assert_eq!(result.text, "heyyy rafaelitewrld shy peek returns");
        assert!(result.fixes.contains(&"unlinked a repeated @handle X already tags".to_owned()));
    }

    #[test]
    fn leaves_a_third_party_tagged_which_may_be_deliberate() {
        assert_eq!(
            replying("say hi to @someoneelse for me", "rafaelitewrld").text,
            "say hi to @someoneelse for me"
        );
    }
}

mod truncate_at_boundary {
    use super::*;

    #[test]
    fn leaves_text_that_already_fits_alone() {
        assert_eq!(truncate_at_boundary("short enough", 100), "short enough");
    }

    // The published failure: a reply ended "that's OpenHum".
    #[test]
    fn cuts_at_a_word_rather_than_through_one() {
        assert_eq!(truncate_at_boundary("we are building OpenHumans together", 20), "we are building");
    }

    // Half a URL is not a shorter receipt, it is a broken one.
    #[test]
    fn drops_a_receipt_link_whole_rather_than_publishing_half_of_it() {
        let text = format!("sent! receipt: https://rh-scan.com/tx/0x{}", "ab".repeat(32));
        let cut = truncate_at_boundary(&text, 40);
        assert_eq!(cut, "sent! receipt:");
        assert!(!cut.contains("rh-scan"));
    }

    #[test]
    fn still_cuts_a_single_unbroken_token_since_half_of_it_is_meaningless_anyway() {
        assert_eq!(truncate_at_boundary(&"a".repeat(50), 10), "a".repeat(10));
    }

    #[test]
    fn counts_length_as_x_and_the_legacy_service_did_in_utf16_units_and_never_splits_a_char() {
        assert_eq!(post_length("gm 🚀"), 5);
        assert_eq!(truncate_at_boundary("🚀🚀🚀", 3), "🚀");
    }
}

mod the_ask {
    use super::*;

    #[test]
    fn sees_a_question_on_the_last_line_only() {
        assert!(asks_the_reader("the chart held.\nwhat are you watching?  \n\n"));
        assert!(!asks_the_reader("what are you watching?\nthe chart held."));
        assert!(!asks_the_reader(""));
    }

    #[test]
    fn cuts_the_last_line_or_turns_a_lone_question_deadpan() {
        assert_eq!(without_the_ask("the chart held.\nwhat are you watching?"), "the chart held.");
        assert_eq!(without_the_ask("what are you watching?"), "what are you watching.");
    }
}
