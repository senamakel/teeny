use teeny_media::moon::{ladder, rung};

use super::*;
use crate::moon::pick::drawn;
use crate::moon::pick::tests::{board, tiny};

#[test]
fn hands_the_model_the_arithmetic_and_the_rules() {
    let token = drawn(&tiny());
    let board: Vec<ListedCoin> = board().iter().map(drawn).collect();
    let target = rung(&token, &board[4]);
    let climb = ladder(&token, &board, &board[4], 3, 2.0);
    let ask = moon_caption_prompt(
        &StageContext {
            persona: "PERSONA".into(),
            bot_username: "TinyMascot".into(),
            today: "2026-09-14".into(),
        },
        &MoonCaptionInput {
            token: &token,
            target: &target,
            ladder: &climb,
            board_name: "AI meme coins",
            board_size: 12,
            history: "Yesterday: $TURBO at 127x",
            community: "",
            max_emoji: 1,
            max_length: 280,
        },
    );
    assert!(ask.system.contains("PERSONA"));
    assert!(ask.system.contains("@TinyMascot"));
    for needle in [
        "$TINY with the market cap of $DRB",
        "37x",
        "#9 of 12 on the board",
        "It ends on the aside, not on a question.",
        "#5 on the board",
        "not a forecast",
        "$DRB is somebody else's token",
        "Yesterday: $TURBO at 127x",
        "At most 1 emoji",
    ] {
        assert!(ask.prompt.contains(needle), "missing {needle}:\n{}", ask.prompt);
    }
}
