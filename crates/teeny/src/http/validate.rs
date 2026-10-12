//! Request bodies checked the way the TypeScript service's zod schemas
//! checked them, with zod 4's issue shapes in `details`. Lengths count
//! UTF-16 code units, as JavaScript's do.

use serde_json::{Map, Value, json};

/// The issues found; empty when the body is good.
pub type Issues = Vec<Value>;

/// zod's name for a value's type.
fn type_name(value: Option<&Value>) -> &'static str {
    match value {
        None => "undefined",
        Some(Value::Null) => "null",
        Some(Value::Bool(_)) => "boolean",
        Some(Value::Number(_)) => "number",
        Some(Value::String(_)) => "string",
        Some(Value::Array(_)) => "array",
        Some(Value::Object(_)) => "object",
    }
}

fn path(key: Option<&str>) -> Value {
    match key {
        Some(key) => json!([key]),
        None => json!([]),
    }
}

fn invalid_type(expected: &str, key: Option<&str>, received: Option<&Value>) -> Value {
    json!({
        "expected": expected,
        "code": "invalid_type",
        "path": path(key),
        "message": format!("Invalid input: expected {expected}, received {}", type_name(received)),
    })
}

/// The body as an object, or the issue zod raised for anything else.
pub fn object<'a>(body: Option<&'a Value>, issues: &mut Issues) -> Option<&'a Map<String, Value>> {
    match body {
        Some(Value::Object(fields)) => Some(fields),
        other => {
            issues.push(invalid_type("object", None, other));
            None
        }
    }
}

/// A string field of `min..=max` UTF-16 units; absent is an issue only when
/// `required`.
pub fn string(
    fields: &Map<String, Value>,
    key: &str,
    min: usize,
    max: usize,
    required: bool,
    issues: &mut Issues,
) -> Option<String> {
    let value = fields.get(key);
    let text = match value {
        Some(Value::String(text)) => text,
        None if !required => return None,
        other => {
            issues.push(invalid_type("string", Some(key), other));
            return None;
        }
    };
    let length = text.encode_utf16().count();
    if length < min {
        issues.push(json!({
            "origin": "string", "code": "too_small", "minimum": min, "inclusive": true,
            "path": path(Some(key)),
            "message": format!("Too small: expected string to have >={min} characters"),
        }));
        return None;
    }
    if length > max {
        issues.push(json!({
            "origin": "string", "code": "too_big", "maximum": max, "inclusive": true,
            "path": path(Some(key)),
            "message": format!("Too big: expected string to have <={max} characters"),
        }));
        return None;
    }
    Some(text.clone())
}

/// One of `values`, or `default` when absent (an issue when there is none).
pub fn one_of(
    fields: &Map<String, Value>,
    key: &str,
    values: &[&str],
    default: Option<&str>,
    issues: &mut Issues,
) -> Option<String> {
    match (fields.get(key), default) {
        (None, Some(default)) => return Some(default.to_owned()),
        (Some(Value::String(value)), _) if values.contains(&value.as_str()) => {
            return Some(value.clone());
        }
        _ => {}
    }
    let options: Vec<String> = values.iter().map(|value| format!("\"{value}\"")).collect();
    issues.push(json!({
        "code": "invalid_value",
        "values": values,
        "path": path(Some(key)),
        "message": format!("Invalid option: expected one of {}", options.join("|")),
    }));
    None
}

#[cfg(test)]
#[path = "validate_tests.rs"]
mod tests;
