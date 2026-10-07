//! The picture steps the serial and the article share with the morning:
//! the seed picture briefed, the reference sheet in front of the prompt, and
//! the vision check that the small yellow robot is Tiny.

use std::sync::Arc;

use serde::Deserialize;
use teeny_media::reference::{ReferenceSeed, ReferenceSheet, TextModel};
use teeny_media::store::StoredMedia;
use teeny_media::types::{ImageInput, ImageReader, MimeType};

use crate::backrooms::ask::{Ask, Shape, ask_json, at_most, extract_json};
use crate::backrooms::text::head;

/// The longest prompt a generator is handed.
pub const MAX_GENERATION_PROMPT: usize = 2_000;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Brief {
    pub brief: String,
}

impl Shape for Brief {
    fn check(&self) -> Result<(), String> {
        if self.brief.encode_utf16().count() < 10 {
            return Err("brief: at least 10 characters".into());
        }
        at_most("brief", &self.brief, 500)
    }
}

pub fn seed_brief_prompt(look: &str) -> Ask {
    Ask::new(
        "You write terse style notes for an image model.",
        [
            "Below is a long description of one official picture of Tiny, a small yellow robot",
            "mascot. Everything about what Tiny always looks like is already known to the model;",
            "write only what is particular to THIS picture, in one plain paragraph of at most 60",
            "words: the pose and expression, the props or clothing, the setting, the palette, the",
            "light, the rendering style and the mood. No headings, no bullets, no markdown.",
            "",
            look,
            "",
            "Shape: {\"brief\": string}",
        ]
        .join("\n"),
    )
}

pub fn check_question(sheet: Option<&str>) -> String {
    [
        "This picture was generated to show Tiny, a mascot.".to_owned(),
        match sheet {
            Some(sheet) => format!("Tiny looks like this: {sheet}"),
            None => "Tiny is a small, round, yellow robot with a simple friendly face.".into(),
        },
        "Does the main character in the picture match that description in body shape and".into(),
        "colour, and is the picture free of readable gibberish text, extra limbs, and".into(),
        "anything disturbing? Answer with one JSON object: {\"ok\": true|false, \"reason\": string},".into(),
        "and nothing else.".into(),
    ]
    .join(" ")
}

/// The sheet, and the seed picture briefed down to what is particular to it.
pub async fn sheet_and_seed(
    reference: Option<&Arc<ReferenceSheet>>,
    text: &dyn TextModel,
    random: f64,
    problems: &mut Vec<String>,
) -> (Option<String>, Option<ReferenceSeed>) {
    let Some(reference) = reference else {
        return (None, None);
    };
    let sheet = reference.get().await;
    let mut seed = reference.seed(random).await;
    if let Some(found) = &seed {
        match ask_json::<Brief>(text, &seed_brief_prompt(&found.look)).await {
            Ok(brief) => {
                seed = Some(ReferenceSeed {
                    look: brief.brief,
                    ..found.clone()
                });
            }
            Err(error) => {
                problems.push(format!(
                    "could not brief the seed picture: {error}; drawing from the sheet alone"
                ));
                seed = None;
            }
        }
    }
    (sheet, seed)
}

#[derive(Deserialize)]
struct Verdict {
    ok: bool,
    #[serde(default)]
    reason: String,
}

/// Whether the picture is Tiny. With a visitor, only Tiny is judged. No
/// reader, or a reader that fails, passes the picture.
pub async fn check_picture(
    reader: Option<&Arc<dyn ImageReader>>,
    media: &StoredMedia,
    sheet: Option<&str>,
    visitor: Option<&str>,
) -> (bool, String) {
    let Some(reader) = reader.filter(|_| media.mime_type != MimeType::Mp4) else {
        return (true, "no vision model to check with".into());
    };
    let mut question = check_question(sheet);
    if let Some(visitor) = visitor {
        question = question.replace(
            "Does the main character in the picture match",
            &format!("A person ({visitor}) is meant to be in the picture too; ignore them entirely. Does the small yellow robot in the picture match"),
        );
    }
    let image = ImageInput::Bytes {
        data: media.data.clone(),
        mime_type: media.mime_type,
    };
    match reader.describe(image, Some(&question)).await {
        Ok(answer) => {
            if let Some(verdict) =
                extract_json(&answer).and_then(|v| serde_json::from_value::<Verdict>(v).ok())
            {
                return (verdict.ok, head(&verdict.reason, 300));
            }
            let lower = answer.trim_start().to_lowercase();
            let ok = if answer.contains("\"ok\": false") || answer.contains("\"ok\":false") {
                false
            } else {
                answer.contains("\"ok\": true")
                    || answer.contains("\"ok\":true")
                    || lower.starts_with("yes")
                    || lower.starts_with("true")
            };
            (ok, head(&answer, 300))
        }
        Err(error) => (true, format!("could not check: {error}")),
    }
}
