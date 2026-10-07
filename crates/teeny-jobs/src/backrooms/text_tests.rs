use super::*;

#[test]
fn strips_html_and_unescapes_entities() {
    assert_eq!(
        strip_html(
            "a<br>b<br/>&gt;&gt;123 &quot;c&quot; &#39;d&#39; <span class=\"quote\">&gt;e</span>"
        ),
        "a\nb\n>>123 \"c\" 'd' >e"
    );
    assert_eq!(unescape_entities("&#x41;&amp;&bogus;"), "A&&bogus;");
}

#[test]
fn picks_the_words_worth_matching_on() {
    assert_eq!(
        keywords(
            "The AI agents escaped from the lab and the door was open",
            6
        ),
        ["ai", "agents", "escaped", "lab", "door", "open"]
    );
    assert!(keywords("to be or", 6).is_empty());
}

#[test]
fn clips_with_an_ellipsis_and_heads_by_characters() {
    assert_eq!(clip("abcdef", 4), "abc…");
    assert_eq!(clip("abc", 4), "abc");
    assert_eq!(head("héllo", 2), "hé");
}
