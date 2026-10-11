//! Port of `legacy/test/text.test.ts`, plus `clip`.

use super::*;

#[test]
fn never_leaves_half_an_emoji_behind() {
    // "ab" then an emoji: two UTF-16 units. A cut at 3 lands between them.
    assert_eq!(cut_at("ab😀cd", 3), "ab");
}

#[test]
fn keeps_a_whole_emoji_when_the_cut_falls_after_it_and_leaves_short_text_alone() {
    assert_eq!(cut_at("ab😀cd", 4), "ab😀");
    assert_eq!(cut_at("short", 10), "short");
    assert_eq!(cut_at("short", 5), "short");
}

#[test]
fn clip_flattens_whitespace_and_marks_a_cut_with_an_ellipsis() {
    assert_eq!(
        clip("  is the\n\nserial   back? ", 280),
        "is the serial back?"
    );
    assert_eq!(clip("abcdef", 4), "abc…");
    // The ellipsis never sits after half a character.
    assert_eq!(clip("ab😀cd", 4), "ab…");
}
