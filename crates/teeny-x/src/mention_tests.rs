use super::*;
use crate::model::XPost;

fn reply(text: &str, to: Option<&str>) -> XPost {
    XPost {
        in_reply_to_user_id: to.map(Into::into),
        ..XPost::new("1", text)
    }
}

#[test]
fn a_handle_typed_after_the_prefix_x_adds_is_deliberate() {
    assert!(mentions_deliberately(
        "@someone Oh hi fren @TinyMascot",
        "TinyMascot"
    ));
    assert!(mentions_deliberately("hey @tinymascot.", "@TinyMascot"));
    assert!(!mentions_deliberately(
        "@someone @TinyMascot Rhyming is a bold choice",
        "TinyMascot"
    ));
    assert!(!mentions_deliberately("@TinyMascot", "TinyMascot"));
    assert!(!mentions_deliberately("hey @TinyMascotFan", "TinyMascot"));
    assert!(!mentions_deliberately(
        "RT @TinyMascot: @someone hello @TinyMascot",
        "TinyMascot"
    ));
    assert!(mentions_deliberately("wow@TinyMascot", "TinyMascot"));
}

#[test]
fn one_level_means_a_mention_or_a_reply_to_the_account() {
    let bot: UserId = "100".into();
    assert!(is_within_one_level(&reply("x", None), None));
    assert!(is_within_one_level(&reply("x", Some("100")), Some(&bot)));
    assert!(!is_within_one_level(&reply("x", Some("999")), Some(&bot)));
    // Unknown identity fails closed.
    assert!(!is_within_one_level(&reply("x", Some("100")), None));
}

#[test]
fn worth_answering_is_either() {
    let bot: UserId = "100".into();
    assert!(is_worth_answering(
        &reply("@a @TinyMascot hi", Some("100")),
        Some(&bot),
        "TinyMascot"
    ));
    assert!(is_worth_answering(
        &reply("@a hi @TinyMascot", Some("999")),
        Some(&bot),
        "TinyMascot"
    ));
    assert!(!is_worth_answering(
        &reply("@a @TinyMascot hi", Some("999")),
        Some(&bot),
        "TinyMascot"
    ));
}
