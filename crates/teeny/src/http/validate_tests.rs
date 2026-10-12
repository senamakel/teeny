use serde_json::{Map, Value, json};

use super::*;

fn fields(value: Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}

#[test]
fn wants_an_object() {
    let mut issues = Issues::new();
    assert!(object(None, &mut issues).is_none());
    assert_eq!(
        issues,
        [
            json!({ "expected": "object", "code": "invalid_type", "path": [], "message": "Invalid input: expected object, received undefined" })
        ]
    );
    let mut issues = Issues::new();
    assert!(object(Some(&json!("text")), &mut issues).is_none());
    assert_eq!(
        issues[0]["message"],
        "Invalid input: expected object, received string"
    );
    let mut issues = Issues::new();
    assert!(object(Some(&json!({ "a": 1 })), &mut issues).is_some());
    assert!(issues.is_empty());
}

#[test]
fn checks_a_string_its_presence_and_its_length_in_utf16_units() {
    let body = fields(json!({ "task": "", "long": "ab", "emoji": "😀", "number": 3 }));
    let mut issues = Issues::new();

    assert_eq!(string(&body, "missing", 1, 10, false, &mut issues), None);
    assert!(issues.is_empty());

    assert_eq!(string(&body, "task", 1, 10, true, &mut issues), None);
    assert_eq!(
        issues.pop().unwrap(),
        json!({ "origin": "string", "code": "too_small", "minimum": 1, "inclusive": true, "path": ["task"], "message": "Too small: expected string to have >=1 characters" })
    );
    assert_eq!(string(&body, "long", 1, 1, true, &mut issues), None);
    assert_eq!(
        issues.pop().unwrap(),
        json!({ "origin": "string", "code": "too_big", "maximum": 1, "inclusive": true, "path": ["long"], "message": "Too big: expected string to have <=1 characters" })
    );
    // One emoji is two UTF-16 units.
    assert_eq!(string(&body, "emoji", 1, 1, true, &mut issues), None);
    assert_eq!(issues.pop().unwrap()["code"], "too_big");
    assert_eq!(string(&body, "missing", 1, 10, true, &mut issues), None);
    assert_eq!(
        issues.pop().unwrap(),
        json!({ "expected": "string", "code": "invalid_type", "path": ["missing"], "message": "Invalid input: expected string, received undefined" })
    );
    assert_eq!(string(&body, "number", 1, 10, true, &mut issues), None);
    assert_eq!(
        issues.pop().unwrap()["message"],
        "Invalid input: expected string, received number"
    );
    assert_eq!(
        string(&body, "long", 1, 10, true, &mut issues).as_deref(),
        Some("ab")
    );
    assert!(issues.is_empty());
}

#[test]
fn checks_an_enum_with_its_default() {
    let body = fields(json!({ "decision": "maybe", "kind": "post" }));
    let mut issues = Issues::new();
    assert_eq!(
        one_of(&body, "kind", &["post", "reply"], None, &mut issues).as_deref(),
        Some("post")
    );
    assert_eq!(
        one_of(&body, "absent", &["post"], Some("post"), &mut issues).as_deref(),
        Some("post")
    );
    assert!(issues.is_empty());
    assert_eq!(
        one_of(
            &body,
            "decision",
            &["approved", "rejected"],
            None,
            &mut issues
        ),
        None
    );
    assert_eq!(
        issues.pop().unwrap(),
        json!({ "code": "invalid_value", "values": ["approved", "rejected"], "path": ["decision"], "message": "Invalid option: expected one of \"approved\"|\"rejected\"" })
    );
    assert_eq!(
        one_of(&body, "absent", &["approved"], None, &mut issues),
        None
    );
    assert_eq!(issues.pop().unwrap()["code"], "invalid_value");
}
