//! Markdown to the shape the X Articles API takes: typed blocks, inline
//! styles as UTF-16 offset ranges, every picture an atomic block pointing at
//! an entity. It handles the subset the pipeline emits; anything else
//! degrades to plain text rather than failing. No `markdown` entity is ever
//! emitted (they carry a 10,000-character cap on X's side).

use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;
use teeny_x::article::{
    ArticleBlock, ArticleEntity, BlockType, ContentState, EntityData, EntityKind, EntityRange,
    EntityValue, InlineStyle, MediaItem, Mutability, Style,
};

/// `![caption](image:s1)`: where a section's picture goes.
pub static IMAGE_PLACEHOLDER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^!\[([^\]]*)\]\(image:([a-z0-9-]+)\)\s*$").expect("valid"));
static DIVIDER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(-{3,}|\*{3,}|_{3,})$").expect("valid"));
static HEADING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(#{1,3})\s+(.*)$").expect("valid"));
static BULLET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*[-*+]\s+(.*)$").expect("valid"));
static NUMBERED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*\d+[.)]\s+(.*)$").expect("valid"));
static QUOTED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^>\s?(.*)$").expect("valid"));

/// A picture X already holds, and its caption.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ImageRef {
    pub media_id: String,
    pub caption: Option<String>,
}

struct Builder {
    blocks: Vec<ArticleBlock>,
    entities: Vec<ArticleEntity>,
    paragraph: Vec<String>,
    quote: Option<String>,
}

impl Builder {
    fn flush(&mut self) {
        if !self.paragraph.is_empty() {
            let text = self.paragraph.join(" ");
            self.blocks.push(text_block(BlockType::Unstyled, &text));
            self.paragraph.clear();
        }
    }

    fn close_quote(&mut self) {
        if let Some(quote) = self.quote.take() {
            self.blocks.push(text_block(BlockType::Blockquote, &quote));
        }
    }

    fn atomic(&mut self, value: EntityValue) {
        let key = self.entities.len() as u32;
        self.entities.push(ArticleEntity {
            key: key.to_string(),
            value,
        });
        self.blocks.push(ArticleBlock {
            text: " ".into(),
            kind: BlockType::Atomic,
            inline_style_ranges: Vec::new(),
            entity_ranges: vec![EntityRange {
                key,
                offset: 0,
                length: 1,
            }],
        });
    }
}

pub fn markdown_to_content_state(
    markdown: &str,
    images: &BTreeMap<String, ImageRef>,
) -> ContentState {
    let mut b = Builder {
        blocks: Vec::new(),
        entities: Vec::new(),
        paragraph: Vec::new(),
        quote: None,
    };
    let mut fenced = false;
    for raw in markdown.replace("\r\n", "\n").split('\n') {
        let line = raw.trim_end();
        if line.trim().starts_with("```") {
            b.flush();
            b.close_quote();
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        if line.trim().is_empty() {
            b.flush();
            b.close_quote();
            continue;
        }
        if let Some(image) = IMAGE_PLACEHOLDER.captures(line.trim()) {
            b.flush();
            b.close_quote();
            if let Some(found) = images.get(&image[2]) {
                let caption = found
                    .caption
                    .clone()
                    .or_else(|| Some(image[1].to_owned()))
                    .filter(|c| !c.is_empty());
                b.atomic(EntityValue {
                    kind: EntityKind::Image,
                    mutability: Mutability::Immutable,
                    data: EntityData {
                        media_items: vec![MediaItem {
                            media_category: "tweet_image".into(),
                            media_id: found.media_id.clone(),
                        }],
                        caption,
                        url: None,
                    },
                });
            }
            continue;
        }
        if DIVIDER.is_match(line.trim()) {
            b.flush();
            b.close_quote();
            b.atomic(EntityValue {
                kind: EntityKind::Divider,
                mutability: Mutability::Immutable,
                data: EntityData {
                    media_items: Vec::new(),
                    caption: None,
                    url: None,
                },
            });
            continue;
        }
        if let Some(heading) = HEADING.captures(line) {
            b.flush();
            b.close_quote();
            // The title travels separately; an H1 in the body would repeat it.
            match heading[1].len() {
                1 => {}
                2 => b.blocks.push(text_block(BlockType::HeaderTwo, &heading[2])),
                _ => b
                    .blocks
                    .push(text_block(BlockType::HeaderThree, &heading[2])),
            }
            continue;
        }
        if let Some(bullet) = BULLET.captures(line) {
            b.flush();
            b.close_quote();
            b.blocks
                .push(text_block(BlockType::UnorderedListItem, &bullet[1]));
            continue;
        }
        if let Some(numbered) = NUMBERED.captures(line) {
            b.flush();
            b.close_quote();
            b.blocks
                .push(text_block(BlockType::OrderedListItem, &numbered[1]));
            continue;
        }
        if let Some(quoted) = QUOTED.captures(line) {
            b.flush();
            b.quote = Some(match b.quote.take() {
                Some(quote) => format!("{quote}\n{}", &quoted[1]),
                None => quoted[1].to_owned(),
            });
            continue;
        }
        b.close_quote();
        b.paragraph.push(line.trim().to_owned());
    }
    b.flush();
    b.close_quote();
    ContentState {
        blocks: b.blocks,
        entities: b.entities,
    }
}

fn text_block(kind: BlockType, source: &str) -> ArticleBlock {
    let (text, ranges) = parse_inline(source);
    ArticleBlock {
        text,
        kind,
        inline_style_ranges: ranges,
        entity_ranges: Vec::new(),
    }
}

struct Open {
    marker: &'static str,
    style: Style,
    at: usize,
}

const MARKERS: [&str; 5] = ["**", "__", "~~", "*", "_"];

/// Inline markers to style ranges, offsets in UTF-16 code units of the text
/// with the markers removed, as X counts them.
pub fn parse_inline(source: &str) -> (String, Vec<InlineStyle>) {
    let chars: Vec<char> = source.chars().collect();
    let mut text: Vec<u16> = Vec::new();
    let mut ranges: Vec<InlineStyle> = Vec::new();
    let mut open: Vec<Open> = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        let rest: String = chars[index..chars.len().min(index + 2)].iter().collect();
        let marker = MARKERS.into_iter().find(|marker| rest.starts_with(marker));
        if let Some(marker) = marker
            && is_marker_here(&chars, index, marker, &open)
        {
            let style = match marker {
                "~~" => Style::Strikethrough,
                "**" | "__" => Style::Bold,
                _ => Style::Italic,
            };
            if let Some(at) = open.iter().position(|item| item.marker == marker) {
                let opened = open.remove(at);
                if text.len() > opened.at {
                    ranges.push(InlineStyle {
                        offset: opened.at as u32,
                        length: (text.len() - opened.at) as u32,
                        style,
                    });
                }
            } else {
                open.push(Open {
                    marker,
                    style,
                    at: text.len(),
                });
            }
            index += marker.chars().count();
            continue;
        }
        let mut buffer = [0u16; 2];
        text.extend_from_slice(chars[index].encode_utf16(&mut buffer));
        index += 1;
    }
    // A marker never closed was punctuation, not a style: put it back.
    for item in open.into_iter().rev() {
        let units: Vec<u16> = item.marker.encode_utf16().collect();
        text.splice(item.at..item.at, units.iter().copied());
        for range in &mut ranges {
            if range.offset as usize >= item.at {
                range.offset += units.len() as u32;
            }
        }
        let _ = item.style;
    }
    ranges.sort_by_key(|range| range.offset);
    (String::from_utf16_lossy(&text), ranges)
}

/// A `_` inside a word or a `*` between spaces is not a marker. An opening
/// marker needs a non-space after it; a closing one a non-space before it.
fn is_marker_here(chars: &[char], index: usize, marker: &str, open: &[Open]) -> bool {
    let before = if index == 0 { ' ' } else { chars[index - 1] };
    let after = chars
        .get(index + marker.chars().count())
        .copied()
        .unwrap_or(' ');
    let closing = open.iter().any(|item| item.marker == marker);
    let word = |c: char| c.is_alphanumeric() || c == '_';
    if (marker == "_" || marker == "__") && word(before) && word(after) {
        return false;
    }
    if closing {
        !before.is_whitespace()
    } else {
        !after.is_whitespace()
    }
}

/// The content state as the legacy record kept it: camelCase keys.
pub fn record_shape(state: &ContentState) -> Value {
    fn camel(value: Value) -> Value {
        match value {
            Value::Object(map) => Value::Object(
                map.into_iter()
                    .map(|(key, value)| {
                        let mut out = String::new();
                        let mut upper = false;
                        for ch in key.chars() {
                            if ch == '_' {
                                upper = true;
                            } else if upper {
                                out.extend(ch.to_uppercase());
                                upper = false;
                            } else {
                                out.push(ch);
                            }
                        }
                        (out, camel(value))
                    })
                    .collect(),
            ),
            Value::Array(items) => Value::Array(items.into_iter().map(camel).collect()),
            other => other,
        }
    }
    camel(serde_json::to_value(state).unwrap_or_default())
}

#[cfg(test)]
#[path = "content_state_tests.rs"]
mod tests;
