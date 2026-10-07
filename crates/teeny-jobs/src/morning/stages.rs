//! One model call that has to come back as a JSON object of a known shape,
//! and the shapes the daily posts ask for. A port of `askJson`, `extractJson`
//! and the zod schemas of `legacy/src/morning/stages.ts`.
//!
//! The stages of the morning post are fixed (pick, draw, script, caption),
//! so each is a plain completion asked for one object rather than a tool an
//! agent may or may not call. Models wrap JSON in prose and code fences; the
//! first balanced object in the answer is what gets parsed. A shape that is
//! refused is sent back once with the complaint, and a second miss is the
//! stage failing. A call that fails outright (a timeout, a 502 from the
//! ladder) is tried once more with the same prompt.

use serde::Serialize;
use serde_json::{Map, Value};
use teeny_media::reference::TextModel;

const INSTRUCTION: &str =
    "\n\nAnswer with one JSON object and nothing else: no prose before or after, no code fence.";

/// Why a stage produced nothing.
#[derive(Debug, thiserror::Error)]
pub enum StageError {
    /// The model could not be reached twice running.
    #[error("{0}")]
    Model(String),
    /// It answered twice, and neither answer was the shape.
    #[error("the model could not produce the expected JSON: {issues}; the answer ended: {tail}")]
    Shape { issues: String, tail: String },
}

/// A shape a stage asks for: read from a JSON object, or the issues with it
/// as `path: message`, the way zod reported them.
pub trait Shape: Sized {
    fn read(value: &Value) -> Result<Self, Vec<String>>;
}

/// Asks `model` for one `T`.
pub async fn ask_json<T: Shape>(
    model: &dyn TextModel,
    system: &str,
    prompt: &str,
) -> Result<T, StageError> {
    let mut asking = format!("{prompt}{INSTRUCTION}");
    let mut last_error = "no answer".to_owned();
    let mut last_raw = String::new();
    let mut failed = false;
    let mut attempt = 0;
    while attempt < 2 {
        let raw = match model.complete(Some(system), &asking).await {
            Ok(raw) => raw,
            Err(error) if failed => return Err(StageError::Model(error.to_string())),
            Err(_) => {
                failed = true;
                continue;
            }
        };
        attempt += 1;
        match extract_json(&raw) {
            None => last_error = "the answer contained no JSON object".into(),
            Some(candidate) => match T::read(&candidate) {
                Ok(shape) => return Ok(shape),
                Err(issues) => last_error = issues.join("; "),
            },
        }
        last_raw = raw;
        asking = format!(
            "{prompt}{INSTRUCTION}\n\nYour previous answer was rejected ({last_error}). Answer again, as one JSON object matching the shape described."
        );
    }
    Err(StageError::Shape {
        issues: last_error,
        tail: serde_json::to_string(&tail_units(&last_raw, 400)).unwrap_or_default(),
    })
}

/// The last `units` UTF-16 units of `text`, as `slice(-n)` took them.
fn tail_units(text: &str, units: usize) -> String {
    let encoded: Vec<u16> = text.encode_utf16().collect();
    let start = encoded.len().saturating_sub(units);
    String::from_utf16_lossy(&encoded[start..])
}

/// The first balanced `{…}` in a string, parsed.
pub fn extract_json(text: &str) -> Option<Value> {
    let start = text.find('{')?;
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut in_string = false;
    let mut index = start;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            if byte == b'\\' {
                index += 1;
            } else if byte == b'"' {
                in_string = false;
            }
        } else if byte == b'"' {
            in_string = true;
        } else if byte == b'{' {
            depth += 1;
        } else if byte == b'}' {
            depth -= 1;
            if depth == 0 {
                return serde_json::from_str(&text[start..=index]).ok();
            }
        }
        index += 1;
    }
    None
}

/// A string's length as JavaScript counts it.
pub(crate) fn js_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// The first `units` UTF-16 units, never splitting a character.
pub(crate) fn js_prefix(text: &str, units: usize) -> String {
    let mut count = 0;
    for (index, ch) in text.char_indices() {
        count += ch.len_utf16();
        if count > units {
            return text[..index].to_owned();
        }
    }
    text.to_owned()
}

fn kind_of(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Reads the fields of one object, collecting every issue.
pub(crate) struct Fields<'a> {
    object: Option<&'a Map<String, Value>>,
    prefix: String,
    pub(crate) issues: Vec<String>,
}

impl<'a> Fields<'a> {
    pub(crate) fn new(value: &'a Value) -> Self {
        let mut fields = Self {
            object: value.as_object(),
            prefix: String::new(),
            issues: Vec::new(),
        };
        if fields.object.is_none() {
            fields.issues.push(format!(
                "(root): Expected object, received {}",
                kind_of(value)
            ));
        }
        fields
    }

    fn nested(value: &'a Value, prefix: String) -> Self {
        let mut fields = Self::new(value);
        fields.prefix = prefix;
        if fields.object.is_none() {
            fields.issues = vec![format!(
                "{}: Expected object, received {}",
                fields.prefix.trim_end_matches('.'),
                kind_of(value)
            )];
        }
        fields
    }

    fn issue(&mut self, key: &str, message: impl std::fmt::Display) {
        self.issues.push(format!("{}{key}: {message}", self.prefix));
    }

    fn get(&self, key: &str) -> Option<&'a Value> {
        self.object
            .and_then(|object| object.get(key))
            .filter(|value| !value.is_null())
    }

    fn raw(&self, key: &str) -> Option<&'a Value> {
        self.object.and_then(|object| object.get(key))
    }

    /// A required string, `min..=max` long.
    pub(crate) fn string(&mut self, key: &str, min: usize, max: usize) -> String {
        match self.raw(key) {
            None => {
                self.issue(key, "Required");
                String::new()
            }
            Some(value) => self.check_string(key, value, min, max),
        }
    }

    fn check_string(&mut self, key: &str, value: &Value, min: usize, max: usize) -> String {
        let Some(text) = value.as_str() else {
            self.issue(
                key,
                format!("Expected string, received {}", kind_of(value)),
            );
            return String::new();
        };
        let length = js_len(text);
        if length < min {
            self.issue(
                key,
                format!("String must contain at least {min} character(s)"),
            );
        } else if length > max {
            self.issue(key, format!("String must contain at most {max} character(s)"));
        }
        text.to_owned()
    }

    /// A string that may be absent (or null), then `default`.
    pub(crate) fn string_or(&mut self, key: &str, max: usize, default: &str) -> String {
        match self.get(key) {
            None => default.to_owned(),
            Some(value) => self.check_string(key, value, 0, max),
        }
    }

    /// A string that may be absent.
    pub(crate) fn optional_string(&mut self, key: &str, max: usize) -> Option<String> {
        self.get(key)
            .map(|value| self.check_string(key, value, 0, max))
    }

    /// A non-negative integer.
    pub(crate) fn count(&mut self, key: &str) -> u32 {
        match self.raw(key) {
            None => {
                self.issue(key, "Required");
                0
            }
            Some(value) => self.check_count(key, value),
        }
    }

    fn check_count(&mut self, key: &str, value: &Value) -> u32 {
        match value.as_f64() {
            None => {
                self.issue(
                    key,
                    format!("Expected number, received {}", kind_of(value)),
                );
                0
            }
            Some(number) if number.fract() != 0.0 => {
                self.issue(key, "Expected integer, received float");
                0
            }
            Some(number) if number < 0.0 => {
                self.issue(key, "Number must be greater than or equal to 0");
                0
            }
            Some(number) => number.min(f64::from(u32::MAX)) as u32,
        }
    }

    /// A non-negative integer or null; the key must be there.
    pub(crate) fn nullable_count(&mut self, key: &str) -> Option<u32> {
        match self.raw(key) {
            None => {
                self.issue(key, "Required");
                None
            }
            Some(Value::Null) => None,
            Some(value) => Some(self.check_count(key, value)),
        }
    }

    pub(crate) fn boolean(&mut self, key: &str) -> bool {
        match self.raw(key) {
            None => {
                self.issue(key, "Required");
                false
            }
            Some(Value::Bool(value)) => *value,
            Some(value) => {
                self.issue(
                    key,
                    format!("Expected boolean, received {}", kind_of(value)),
                );
                false
            }
        }
    }

    /// One of `options`.
    pub(crate) fn one_of(&mut self, key: &str, options: &[&str]) -> String {
        let text = match self.raw(key) {
            None => {
                self.issue(key, "Required");
                return String::new();
            }
            Some(value) => match value.as_str() {
                Some(text) => text.to_owned(),
                None => {
                    self.issue(
                        key,
                        format!("Expected string, received {}", kind_of(value)),
                    );
                    return String::new();
                }
            },
        };
        if !options.contains(&text.as_str()) {
            let expected = options
                .iter()
                .map(|option| format!("'{option}'"))
                .collect::<Vec<_>>()
                .join(" | ");
            self.issue(
                key,
                format!("Invalid enum value. Expected {expected}, received '{text}'"),
            );
        }
        text
    }

    /// A list that may be absent (then empty), of `min..=max` items each read by `item`.
    pub(crate) fn list<T>(
        &mut self,
        key: &str,
        min: usize,
        max: usize,
        mut item: impl FnMut(&mut Self, &str, &'a Value) -> T,
    ) -> Vec<T> {
        let Some(value) = self.get(key) else {
            return Vec::new();
        };
        let Some(items) = value.as_array() else {
            self.issue(key, format!("Expected array, received {}", kind_of(value)));
            return Vec::new();
        };
        if items.len() < min {
            self.issue(key, format!("Array must contain at least {min} element(s)"));
        } else if items.len() > max {
            self.issue(key, format!("Array must contain at most {max} element(s)"));
        }
        items
            .iter()
            .enumerate()
            .map(|(index, value)| item(self, &format!("{key}.{index}"), value))
            .collect()
    }

    /// A string inside a list.
    pub(crate) fn item_string(&mut self, path: &str, value: &Value, min: usize, max: usize) -> String {
        self.check_string(path, value, min, max)
    }

    /// An object inside a list, read by `read`.
    pub(crate) fn item_object<T>(
        &mut self,
        path: &str,
        value: &'a Value,
        read: impl FnOnce(&mut Fields<'a>) -> T,
    ) -> T {
        let mut nested = Fields::nested(value, format!("{}{path}.", self.prefix));
        let read = read(&mut nested);
        self.issues.append(&mut nested.issues);
        read
    }

    pub(crate) fn finish<T>(self, value: T) -> Result<T, Vec<String>> {
        if self.issues.is_empty() {
            Ok(value)
        } else {
            Err(self.issues)
        }
    }
}

// --- the shapes --------------------------------------------------------------

/// The topics a morning post may be about.
pub const TOPICS: [&str; 4] = ["ai", "crypto", "tech", "culture"];

/// One of the picker's three best candidates, and what people would type back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Shortlisted {
    pub story: u32,
    pub talk: String,
}

/// Which story the morning is about: a 1-based index into the list the model
/// was shown (the news and the timeline as one), or 0 with a `fallback`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Pick {
    pub shortlist: Vec<Shortlisted>,
    pub story: u32,
    pub headline: String,
    /// One of [`TOPICS`].
    pub topic: String,
    pub hook: String,
    pub angle: String,
    pub visual: String,
    pub fallback: Option<String>,
}

impl Shape for Pick {
    fn read(value: &Value) -> Result<Self, Vec<String>> {
        let mut fields = Fields::new(value);
        let shortlist = fields.list("shortlist", 0, 3, |fields, path, value| {
            fields.item_object(path, value, |item| Shortlisted {
                story: item.count("story"),
                // Three sample replies run long; a long one is cut, not refused.
                talk: js_prefix(&item.string("talk", 0, usize::MAX), 300),
            })
        });
        let pick = Self {
            shortlist,
            story: fields.count("story"),
            headline: fields.string("headline", 1, 200),
            topic: fields.one_of("topic", &TOPICS),
            hook: fields.string_or("hook", 300, ""),
            angle: fields.string("angle", 1, 300),
            visual: fields.string("visual", 1, 300),
            fallback: fields.optional_string("fallback", 300),
        };
        fields.finish(pick)
    }
}

/// The picture: the story as a scene with Tiny in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Scene {
    /// The ideas considered before one was chosen, kept for the journal.
    pub concepts: Vec<String>,
    pub scene: String,
    pub story_cue: String,
    pub alt: String,
}

impl Shape for Scene {
    fn read(value: &Value) -> Result<Self, Vec<String>> {
        let mut fields = Fields::new(value);
        let scene = Self {
            concepts: fields.list("concepts", 1, 5, |fields, path, value| {
                fields.item_string(path, value, 5, 300)
            }),
            scene: fields.string("scene", 20, 800),
            story_cue: fields.string("storyCue", 5, 300),
            alt: fields.string("alt", 5, 200),
        };
        fields.finish(scene)
    }
}

/// The clip: a shot list, and what a viewer will see, in a line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Script {
    pub script: String,
    pub summary: String,
}

impl Shape for Script {
    fn read(value: &Value) -> Result<Self, Vec<String>> {
        let mut fields = Fields::new(value);
        let script = Self {
            script: fields.string("script", 40, 1_200),
            summary: fields.string("summary", 10, 200),
        };
        fields.finish(script)
    }
}

/// A caption: the moon's and the morning's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Caption {
    pub text: String,
}

impl Shape for Caption {
    fn read(value: &Value) -> Result<Self, Vec<String>> {
        let mut fields = Fields::new(value);
        let caption = Self {
            text: fields.string("text", 1, 400),
        };
        fields.finish(caption)
    }
}

/// Whether a subject or a post is fresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Fresh,
    Repeat,
    Stale,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::Repeat => "repeat",
            Self::Stale => "stale",
        }
    }
}

/// Memory asked what the account already said, and a model reading it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Freshness {
    pub said: String,
    pub reason: String,
    pub verdict: Verdict,
}

impl Shape for Freshness {
    fn read(value: &Value) -> Result<Self, Vec<String>> {
        let mut fields = Fields::new(value);
        let said = fields.string_or("said", 400, "");
        let reason = fields.string("reason", 0, 300);
        let verdict = match fields.one_of("verdict", &["fresh", "repeat", "stale"]).as_str() {
            "repeat" => Verdict::Repeat,
            "stale" => Verdict::Stale,
            _ => Verdict::Fresh,
        };
        fields.finish(Self {
            said,
            reason,
            verdict,
        })
    }
}

/// One seed picture, boiled down to what is particular about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedBrief {
    pub brief: String,
}

impl Shape for SeedBrief {
    fn read(value: &Value) -> Result<Self, Vec<String>> {
        let mut fields = Fields::new(value);
        let brief = Self {
            brief: fields.string("brief", 10, 500),
        };
        fields.finish(brief)
    }
}

/// The vision ladder's verdict on a drawn picture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub ok: bool,
    pub reason: String,
}

impl Shape for Check {
    fn read(value: &Value) -> Result<Self, Vec<String>> {
        let mut fields = Fields::new(value);
        let check = Self {
            ok: fields.boolean("ok"),
            reason: fields.string_or("reason", usize::MAX, ""),
        };
        fields.finish(check)
    }
}

#[cfg(test)]
#[path = "stages_tests.rs"]
mod tests;
