use super::*;

#[test]
fn bounds_a_number_the_way_the_typescript_did() {
    // Missing, unparseable, empty and zero all fall back to the default.
    assert_eq!(bounded(None, 40, 1, 200), 40);
    assert_eq!(bounded(Some("abc"), 40, 1, 200), 40);
    assert_eq!(bounded(Some(""), 40, 1, 200), 40);
    assert_eq!(bounded(Some("0"), 40, 1, 200), 40);
    // Anything else is clamped into range.
    assert_eq!(bounded(Some("5"), 40, 1, 200), 5);
    assert_eq!(bounded(Some(" 7 "), 40, 1, 200), 7);
    assert_eq!(bounded(Some("-3"), 40, 1, 200), 1);
    assert_eq!(bounded(Some("9999"), 40, 1, 200), 200);
    assert_eq!(bounded(Some("1e2"), 40, 1, 200), 100);
    assert_eq!(bounded(Some("2.9"), 40, 1, 200), 2);
}
