use super::*;

#[test]
fn an_id_is_its_string_on_the_wire() {
    let id = PostId::from("1834567890123456789");
    assert_eq!(
        serde_json::to_string(&id).unwrap(),
        "\"1834567890123456789\""
    );
    let back: PostId = serde_json::from_str("\"1834567890123456789\"").unwrap();
    assert_eq!(back, id);
}

#[test]
fn an_id_prints_as_itself() {
    let id = UserId::from(String::from("42"));
    assert_eq!(id.to_string(), "42");
    assert_eq!(id.as_str(), "42");
}
