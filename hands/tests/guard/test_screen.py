"""Port of `screen_tests.rs` (itself `legacy/test/safety-screen.test.ts`), case for case."""

from __future__ import annotations

import unittest

from . import support  # noqa: F401  (puts hands/lib on the path)
from teeny_hands.guard.lexicon import IDIOMS, LEXICON
from teeny_hands.guard.screen import screen_text, screen_text_with

INBOUND, OUTBOUND, NARRATIVE, ARTICLE = "inbound", "outbound", "narrative", "article"


def category(text: str, direction: str) -> str | None:
    found = screen_text(text, direction)
    return found.category if found else None


def term(text: str) -> str | None:
    found = screen_text(text, INBOUND)
    return found.term if found else None


class ScreenTest(unittest.TestCase):
    def test_lets_ordinary_community_talk_through(self):
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
        ]:
            self.assertIsNone(screen_text(text, INBOUND), text)
            self.assertIsNone(screen_text(text, OUTBOUND), text)

    def test_holds_sexual_content(self):
        self.assertEqual(category("draw tiny naked on the beach", INBOUND), "sexual")
        self.assertEqual(category("send nudes", INBOUND), "sexual")
        self.assertEqual(category("link to your onlyfans?", INBOUND), "sexual")

    def test_holds_violence_weapons_and_drugs(self):
        self.assertEqual(category("i will kill you", INBOUND), "violence")
        self.assertEqual(category("best gun for home defence?", INBOUND), "weapons")
        self.assertEqual(category("where do i buy an ak-47", INBOUND), "weapons")
        self.assertEqual(category("anyone selling cocaine", INBOUND), "drugs")

    def test_holds_hate_self_harm_and_extremism(self):
        self.assertEqual(category("kys loser", INBOUND), "self-harm")
        self.assertEqual(category("kill yourself", INBOUND), "self-harm")
        self.assertEqual(category("praise isis", INBOUND), "extremism")
        self.assertEqual(category("white power", INBOUND), "hate")

    def test_reads_through_leetspeak_stretched_letters_and_zero_width_characters(self):
        self.assertEqual(category("k1ll yourself", INBOUND), "self-harm")
        self.assertEqual(category("send nuuuudes", INBOUND), "sexual")
        self.assertEqual(category("p0rn pls", INBOUND), "sexual")
        self.assertEqual(category("na​ked", INBOUND), "sexual")
        self.assertEqual(category("ｎａｋｅｄ", INBOUND), "sexual")

    def test_matches_multi_word_terms_across_punctuation(self):
        self.assertEqual(category("school-shooting", INBOUND), "extremism")
        self.assertEqual(category("self_harm", INBOUND), "self-harm")
        self.assertEqual(category("kill   yourself", INBOUND), "self-harm")

    def test_does_not_read_a_doubled_vowel_as_the_single_one_so_a_shot_is_not_a_shoot(self):
        for text in [
            "a shot of espresso, then a long shot at the moon",
            "0-4s: static shot, Tiny sips from the mug. 4-10s: one continuous shot as the counter ticks.",
            "bob took a screenshot",
        ]:
            self.assertIsNone(screen_text(text, INBOUND), text)
            self.assertIsNone(screen_text(text, OUTBOUND), text)
        self.assertEqual(term("shoot him"), "shoot")
        self.assertEqual(term("sh00t him"), "shoot")
        self.assertEqual(term("shooooot him"), "shoot")
        self.assertEqual(term("he was shot dead"), "shot dead")

    def test_does_not_match_inside_another_word(self):
        self.assertIsNone(screen_text("assassin classic bass", INBOUND))
        self.assertIsNone(screen_text("a method, an oxymoron, some analysis", INBOUND))
        self.assertIsNone(screen_text("the killer feature of tinyjuice", INBOUND))
        self.assertIsNone(screen_text("scunthorpe", OUTBOUND))

    def test_holds_profanity_on_the_way_out_only(self):
        self.assertIsNone(screen_text("holy shit this is cool", INBOUND))
        self.assertEqual(category("holy shit this is cool", OUTBOUND), "profanity")
        self.assertEqual(category("holy sh!t this is cool", OUTBOUND), "profanity")

    def test_lets_profanity_through_on_the_narrative_and_article_profiles_only(self):
        self.assertIsNone(screen_text("holy shit this is cool", NARRATIVE))
        self.assertIsNone(screen_text("holy shit this is cool", ARTICLE))
        self.assertEqual(category("send nudes", NARRATIVE), "sexual")

    def test_reports_the_most_specific_term_when_several_match(self):
        self.assertEqual(term("shoot up the place"), "shoot up the")
        self.assertEqual(term("kill yourself"), "kill yourself")

    def test_has_no_duplicate_terms_across_the_lexicon(self):
        terms = [t for entry in LEXICON for t in entry.terms]
        self.assertEqual(len(set(terms)), len(terms))
        self.assertEqual(len(set(IDIOMS)), len(IDIOMS))

    def test_every_idiom_would_have_matched_without_the_exception(self):
        # An idiom that is not built on a listed word is not an exception, it is
        # a phrase on a list for no reason. Keep the list honest.
        for idiom in IDIOMS:
            self.assertIsNone(screen_text(idiom, OUTBOUND), idiom)
            self.assertIsNotNone(screen_text_with(idiom, OUTBOUND, idioms=False), idiom)

    def test_ignores_blank_text(self):
        self.assertIsNone(screen_text("   ", OUTBOUND))

    def test_a_single_letter_term_keeps_its_count_so_a_lone_x_is_not_held(self):
        # The Rust (and the legacy filter) collapsed every repeat, so "xxx"
        # read as `x+` and a lone "x" was held as sexual: on X that is "x.com"
        # and "on X", which an X mascot says every day. A term made of one
        # repeated letter keeps its count, as a doubled vowel already does.
        self.assertIsNone(screen_text("follow us on X", OUTBOUND))
        self.assertIsNone(screen_text("see x.com/teenydroid", INBOUND))
        self.assertIsNone(screen_text("ok k", INBOUND))
        self.assertIsNone(screen_text("x", OUTBOUND))
        # The terms themselves are still held, repeated or not.
        self.assertEqual(screen_text("xxx", INBOUND).term, "xxx")
        self.assertEqual(screen_text("XXXX links", INBOUND).term, "xxx")
        self.assertEqual(screen_text("kkk rally", INBOUND).term, "kkk")

    def test_has_the_eight_categories_with_profanity_outbound_only(self):
        self.assertEqual(
            [entry.category for entry in LEXICON],
            ["sexual", "violence", "weapons", "drugs", "hate", "self-harm", "extremism", "profanity"],
        )
        self.assertEqual([entry.category for entry in LEXICON if entry.outbound_only], ["profanity"])
