//! One JSON answer from a plain text model: the legacy `askJson`.
//!
//! The answer is the first balanced `{…}` in the text, read into the shape
//! and held to its few rules ([`Shape::check`]); a miss is asked once more
//! with the reason in front of the model, then it is an error.

use serde::de::DeserializeOwned;
use teeny_media::reference::TextModel;

/// A system line and a prompt.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Ask {
    pub system: String,
    pub prompt: String,
}

impl Ask {
    pub fn new(system: impl Into<String>, prompt: impl Into<String>) -> Self {
        Self {
            system: system.into(),
            prompt: prompt.into(),
        }
    }

    /// The same system line with more prompt after it.
    pub fn followed_by(&self, more: &str) -> Self {
        Self {
            system: self.system.clone(),
            prompt: format!("{}{more}", self.prompt),
        }
    }
}

/// An answer's shape: what serde reads, and the rules it cannot.
pub trait Shape: DeserializeOwned {
    /// Why the answer breaks a rule the schema has, or `Ok`.
    fn check(&self) -> Result<(), String> {
        Ok(())
    }
}

const INSTRUCTION: &str =
    "\n\nAnswer with one JSON object and nothing else: no prose before or after, no code fence.";

/// Asks `model` for one JSON object of shape `T`.
pub async fn ask_json<T: Shape>(model: &dyn TextModel, ask: &Ask) -> anyhow::Result<T> {
    let mut prompt = format!("{}{INSTRUCTION}", ask.prompt);
    let mut last_error = "no answer".to_owned();
    let mut last_raw = String::new();
    let mut failures = 0;
    let mut attempt = 0;
    while attempt < 2 {
        let raw = match model.complete(Some(&ask.system), &prompt).await {
            Ok(raw) => raw,
            Err(error) => {
                failures += 1;
                if failures > 1 {
                    return Err(anyhow::anyhow!(error.to_string()));
                }
                continue;
            }
        };
        attempt += 1;
        match extract_json(&raw) {
            None => last_error = "the answer contained no JSON object".into(),
            Some(value) => match serde_json::from_value::<T>(value) {
                Ok(parsed) => match parsed.check() {
                    Ok(()) => return Ok(parsed),
                    Err(error) => last_error = error,
                },
                Err(error) => last_error = error.to_string(),
            },
        }
        last_raw = raw;
        prompt = format!(
            "{}{INSTRUCTION}\n\nYour previous answer was rejected ({last_error}). Answer again, as one JSON object matching the shape described.",
            ask.prompt
        );
    }
    let tail: String = {
        let chars: Vec<char> = last_raw.chars().collect();
        chars[chars.len().saturating_sub(400)..].iter().collect()
    };
    anyhow::bail!(
        "the model could not produce the expected JSON: {last_error}; the answer ended: {tail:?}"
    )
}

/// The first balanced `{…}` in a string, parsed.
pub fn extract_json(text: &str) -> Option<serde_json::Value> {
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

/// `field` is present and not blank.
pub fn required(field: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("{field}: required"))
    } else {
        Ok(())
    }
}

/// `field` is at most `max` characters, counted as the legacy counted.
pub fn at_most(field: &str, value: &str, max: usize) -> Result<(), String> {
    if value.encode_utf16().count() > max {
        Err(format!("{field}: at most {max} characters"))
    } else {
        Ok(())
    }
}

/// `field` has at most `max` items.
pub fn at_most_items<T>(field: &str, items: &[T], max: usize) -> Result<(), String> {
    if items.len() > max {
        Err(format!("{field}: at most {max} items"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "ask_tests.rs"]
mod tests;
