//! The text steps of an article, as prompts and the shapes they answer in.
//! A structure is written and checked before a word of prose exists, then
//! each section against its own purpose with the earlier ones in view, then
//! the whole is read front to back for flow. Nothing here calls a model.

use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::backrooms::BackroomsRun;
use crate::backrooms::ask::{Ask, Shape, at_most, required};
use crate::backrooms::distill::Thought;
use crate::backrooms::evaluate::PRICE;
use crate::backrooms::variants::{VOICE, material};

pub fn article_voice() -> String {
    format!(
        "{VOICE} you are writing a long piece for teeny's blog, published as an article on x. lore only: nothing about price, market cap, or what anyone should buy. the reader is a stranger who has never seen the lab; they should be able to follow it on the first read."
    )
}

/// What every prompt is written from.
#[derive(Debug, Clone, PartialEq)]
pub struct ArticleMaterial {
    pub run: BackroomsRun,
    pub thought: Thought,
    /// The engine tonight is about, one of the lore's.
    pub engine: Option<String>,
    /// An editorial note the piece is held to.
    pub brief: Option<String>,
    /// The lore as teeny may know it.
    pub lore: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageBrief {
    pub brief: String,
    #[serde(default)]
    pub caption: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Section {
    #[serde(default)]
    pub key: String,
    pub heading: String,
    pub purpose: String,
    pub beats: Vec<String>,
    pub words: u32,
    pub image: ImageBrief,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Structure {
    pub reasons: String,
    pub title: String,
    pub dek: String,
    pub angle: String,
    pub cover: ImageBrief,
    pub sections: Vec<Section>,
    pub closing: String,
}

impl Shape for Structure {
    fn check(&self) -> Result<(), String> {
        required("reasons", &self.reasons)?;
        required("title", &self.title)?;
        at_most("title", &self.title, 80)?;
        required("dek", &self.dek)?;
        at_most("dek", &self.dek, 240)?;
        required("angle", &self.angle)?;
        required("closing", &self.closing)?;
        required("cover.brief", &self.cover.brief)?;
        at_most("cover.caption", &self.cover.caption, 200)?;
        if !(3..=5).contains(&self.sections.len()) {
            return Err("sections: three to five".into());
        }
        for section in &self.sections {
            required("sections.heading", &section.heading)?;
            at_most("sections.heading", &section.heading, 80)?;
            required("sections.purpose", &section.purpose)?;
            required("sections.image.brief", &section.image.brief)?;
            at_most("sections.image.caption", &section.image.caption, 200)?;
            if !(2..=8).contains(&section.beats.len()) {
                return Err("sections.beats: two to eight".into());
            }
            if !(80..=800).contains(&section.words) {
                return Err("sections.words: 80 to 800".into());
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructureCheck {
    pub ok: bool,
    #[serde(default)]
    pub problems: Vec<String>,
    #[serde(default)]
    pub fixes: Vec<String>,
}

impl Shape for StructureCheck {}

/// Section keys are positional whatever the model wrote, and word targets
/// sum to the target. The voice is lowercase; a plan in title case is not.
pub fn normalise_structure(structure: Structure, words: u32) -> Structure {
    let sum: u32 = structure
        .sections
        .iter()
        .map(|s| s.words)
        .sum::<u32>()
        .max(1);
    let scale = f64::from(words) / f64::from(sum);
    Structure {
        title: structure.title.trim().to_lowercase(),
        dek: structure.dek.trim().to_lowercase(),
        cover: ImageBrief {
            caption: structure.cover.caption.trim().to_lowercase(),
            ..structure.cover
        },
        sections: structure
            .sections
            .into_iter()
            .enumerate()
            .map(|(index, section)| Section {
                key: format!("s{}", index + 1),
                words: ((f64::from(section.words) * scale).round() as u32).max(80),
                image: ImageBrief {
                    caption: section.image.caption.trim().to_lowercase(),
                    ..section.image
                },
                ..section
            })
            .collect(),
        ..structure
    }
}

pub struct Planning {
    pub sections_max: u32,
    pub words: u32,
}

const PLAN_RULES: &str = "plan an article of about {words} words as one story that happened in the lab, told in
order, that ends on the turn. three to {max} sections, each a scene with one
purpose, two to five beats that can be shown rather than explained, a word target (the targets
add up to {words}), and one image brief: the single frame an image model draws for
that scene, specific about the room, the light, where teeny is, what the tiny agents carry;
say \"teeny\" and \"the tiny agents\" and nothing about what they look like (the image model
has a sheet for that; a brief that describes a mug or a box gets a mug or a box drawn);
no text, letters, numbers or writing anywhere in the frame. the cover is the whole piece in
one frame, the same rules, composed for a wide 16:9 frame: everything that matters in the
middle band of the picture, room above and below that can be cut away. section headings are
a few words from the story, lowercase, the way a chapter is named; never a number, a count
or a label. the title is lowercase, under 60 characters, no colon. the dek is
one sentence a stranger reads before deciding to click. the angle says what the piece is
about and what the turn is. the closing is the last image the reader is left with.

reasons first: which idea from the room you built the piece on and why.

Shape: {\"reasons\":\"…\",\"title\":\"…\",\"dek\":\"…\",\"angle\":\"…\",\"cover\":{\"brief\":\"…\",\"caption\":\"…\"},
\"sections\":[{\"key\":\"s1\",\"heading\":\"…\",\"purpose\":\"…\",\"beats\":[\"…\",\"…\"],\"words\":300,
\"image\":{\"brief\":\"…\",\"caption\":\"…\"}}],\"closing\":\"…\"}";

pub fn structure_prompt(m: &ArticleMaterial, options: &Planning) -> Ask {
    let mut prompt = material(&m.run, &m.thought, 8);
    prompt.push(String::new());
    if let Some(engine) = &m.engine {
        prompt.extend([
            format!("tonight the story is about {engine}"),
            "(that is a note to you, the writer, about which part of the lore to lean on; the words \"engine\", \"tonight\" and \"the others rest\" are not in the story.)".into(),
            String::new(),
        ]);
    }
    if let Some(brief) = &m.brief {
        prompt.extend([
            "the brief for this piece, from the editor:".into(),
            brief.clone(),
            String::new(),
        ]);
    }
    prompt.extend([
        "what you may draw on (and nothing that has not come out yet):".into(),
        m.lore.clone(),
        String::new(),
        PLAN_RULES
            .replace("{words}", &options.words.to_string())
            .replace("{max}", &options.sections_max.to_string()),
    ]);
    Ask::new(
        format!(
            "{} right now you are planning the piece, not writing it.",
            article_voice()
        ),
        prompt.join("\n"),
    )
}

pub fn check_structure_prompt(structure: &Structure, m: &ArticleMaterial) -> Ask {
    let mut prompt = vec![
        "the plan:".to_owned(),
        render_structure(structure),
        String::new(),
    ];
    if let Some(brief) = &m.brief {
        prompt.extend([
            "the brief the plan must serve:".into(),
            brief.clone(),
            String::new(),
        ]);
    }
    prompt.extend([
        "what came through the wire, which the piece may use freely (real places, companies and".into(),
        "institutions in the news are fine; real people only as obvious satire):".into(),
        m.run.seed.headline(400),
        String::new(),
        "the lore it must stay inside:".into(),
        m.lore.clone(),
        String::new(),
        "answer these, and only these: does it have a beginning, a turn and an ending, in that order?".into(),
        "do any two sections do the same job, or tell the same beat twice? is every beat something".into(),
        "that can be shown in a scene rather than explained? does it contradict the lore above, or".into(),
        "state something about the lab, the cast or the goals that the lore rules out? (new small".into(),
        "objects, notes, rooms and scenes of its own are fine: that is what a story is.) does it give".into(),
        "a price, a market cap, or advice about buying anything? is the title lowercase and under 60".into(),
        "characters, and is every heading words from the story rather than a number or a label? ok".into(),
        "is true only when every answer is the right one. problems are what is wrong, one line each;".into(),
        "fixes are what to change, one line each.".into(),
        String::new(),
        r#"Shape: {"ok":true,"problems":[],"fixes":[]}"#.into(),
    ]);
    Ask::new(
        "you are a cold, exact editor reading the plan for a piece of serial fiction before anyone writes it. you do not write; you say whether the plan will hold and what to change.",
        prompt.join("\n"),
    )
}

pub fn revise_structure_prompt(
    structure: &Structure,
    check: &StructureCheck,
    m: &ArticleMaterial,
    options: &Planning,
) -> Ask {
    let base = structure_prompt(m, options);
    let mut prompt = vec![
        "your plan was:".to_owned(),
        render_structure(structure),
        String::new(),
        "the editor said:".into(),
    ];
    prompt.extend(check.problems.iter().map(|line| format!("- {line}")));
    prompt.extend(check.fixes.iter().map(|line| format!("- fix: {line}")));
    prompt.extend([
        String::new(),
        "plan it again with those fixed and everything else kept: the same title unless the editor".into(),
        "asked for another, the same headings (words from the story, never numbers) unless a section".into(),
        "changed, the same rules as the first time.".into(),
        String::new(),
        base.prompt[base.prompt.find("plan an article").unwrap_or(0)..].to_owned(),
    ]);
    Ask::new(base.system, prompt.join("\n"))
}

pub fn section_prompt(
    structure: &Structure,
    index: usize,
    previous: &[String],
    m: &ArticleMaterial,
) -> Ask {
    let section = &structure.sections[index];
    let mut prompt = vec![
        format!("the piece: \"{}\" — {}", structure.title, structure.dek),
        format!("the angle: {}", structure.angle),
    ];
    if let Some(brief) = &m.brief {
        prompt.push(format!("the brief: {brief}"));
    }
    prompt.extend([
        format!(
            "the closing image, which the last section lands on: {}",
            structure.closing
        ),
        String::new(),
        "the thought it came from, in your words:".into(),
        m.thought.thought.clone(),
        String::new(),
    ]);
    if !previous.is_empty() {
        prompt.push("what is already written, in order (do not repeat any of it):".into());
        for (at, body) in previous.iter().enumerate() {
            prompt.extend([
                format!("## {}", structure.sections[at].heading),
                body.clone(),
                String::new(),
            ]);
        }
        prompt.push(String::new());
    }
    prompt.extend([
        format!(
            "write section {} of {}: \"{}\".",
            index + 1,
            structure.sections.len(),
            section.heading
        ),
        format!("its purpose: {}", section.purpose),
        "its beats, in order:".into(),
    ]);
    prompt.extend(section.beats.iter().map(|beat| format!("- {beat}")));
    prompt.extend([
        String::new(),
        format!("about {} words. markdown, whole paragraphs, cause and effect, in teeny's voice.", section.words),
        "no heading line (it is added), no image (it is added), no list unless the beats are a list,".into(),
        "no sub-headings, no summary of what came before, no explaining the joke. the beats are what".into(),
        "happens; write them as scenes with the objects in them.".into(),
        String::new(),
        "answer with the section's text and nothing else: no title, no preamble, no code fence.".into(),
    ]);
    Ask::new(
        format!(
            "{} you are writing one section of a piece you have already planned.",
            article_voice()
        ),
        prompt.join("\n"),
    )
}

pub fn tighten_prompt(structure: &Structure, index: usize, body: &str, target: u32) -> Ask {
    let section = &structure.sections[index];
    Ask::new(
        format!("{} you are cutting a section you wrote down to length.", article_voice()),
        [
            format!("section \"{}\" is {} words; it should be about {target}.", section.heading, word_count(body)),
            "cut it to that length. keep every beat, keep the order, keep the voice; lose repetition,".into(),
            "throat-clearing and the sentences that explain what a scene already showed. markdown, no".into(),
            "heading line, no image.".into(),
            String::new(),
            body.to_owned(),
            String::new(),
            "answer with the cut section's text and nothing else: no title, no preamble, no code fence.".into(),
        ]
        .join("\n"),
    )
}

/// The flow editor's read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flow {
    pub ok: bool,
    #[serde(default)]
    pub problems: Vec<String>,
    #[serde(default)]
    pub plan: Vec<String>,
}

impl Shape for Flow {}

pub fn flow_read_prompt(markdown: &str, structure: &Structure, m: &ArticleMaterial) -> Ask {
    let mut prompt = vec![
        format!("the angle: {}", structure.angle),
        format!("the closing image it should land on: {}", structure.closing),
    ];
    if let Some(brief) = &m.brief {
        prompt.push(format!("the brief: {brief}"));
    }
    prompt.extend([
        String::new(),
        "the piece:".into(),
        String::new(),
        markdown.to_owned(),
        String::new(),
        "read it once as a stranger. then go back and hunt: list every action, discovery, object or line that happens in two sections (a character looking somewhere twice as if for the first time, a fact learned twice, a description repeated); each is a break. then answer: does each section pick up where the last one left off, or restart the night? is there a jump a reader would trip on: a place, a time, a person appearing with no bridge? does the opening earn the first section and does the last section land on the closing image? does the piece keep one line of time or explain when it moves? a draft written a section at a time almost never reads as one story on the first pass, so ok is rare and earned: true only when you found nothing told twice and no break. problems name the section and the break, one line each, quoting the words. plan says what to do, in reading order, one line each: which telling to keep, which to cut, what bridge to write; keep it to the flow, not a rewrite of the story.".into(),
        String::new(),
        r#"Shape: {"ok":true,"problems":[],"plan":[]}"#.into(),
    ]);
    Ask::new(
        "you are a cold, exact editor reading a piece of serial fiction front to back for the first time. you read for flow only: does one thing lead to the next, does the reader always know where they are, is anything told twice, does the piece arrive where it was going. you do not rewrite; you say where the thread breaks and what to do.",
        prompt.join("\n"),
    )
}

pub fn flow_rewrite_prompt(
    markdown: &str,
    flow: &Flow,
    structure: &Structure,
    m: &ArticleMaterial,
) -> Ask {
    let mut prompt = vec![
        format!("the angle: {}", structure.angle),
        format!("the closing image: {}", structure.closing),
    ];
    if let Some(brief) = &m.brief {
        prompt.push(format!("the brief: {brief}"));
    }
    prompt.extend([String::new(), "the editor said:".into()]);
    prompt.extend(flow.problems.iter().map(|line| format!("- {line}")));
    prompt.extend(flow.plan.iter().map(|line| format!("- do: {line}")));
    prompt.extend([
        String::new(),
        "the piece as it stands:".into(),
        String::new(),
        markdown.to_owned(),
        String::new(),
        "rewrite the whole piece so it reads as one story, front to back: every section picks up where the last one left off, nothing is told twice, every move in time or place has its bridge, and the last section lands on the closing image. keep every scene, object and claim; keep the voice; keep it about the same length. keep every `## ` heading line and every `![…](image:…)` line exactly as they are and where they are; keep the first italic line (the dek) as the first line. answer with the whole rewritten markdown and nothing else: no preamble, no notes, no code fence.".into(),
    ]);
    Ask::new(
        format!(
            "{} you are rewriting a piece you wrote for flow, after an editor read it front to back.",
            article_voice()
        ),
        prompt.join("\n"),
    )
}

/// The plan as a reader sees it.
pub fn render_structure(structure: &Structure) -> String {
    let mut lines = vec![
        format!("title: {}", structure.title),
        format!("dek: {}", structure.dek),
        format!("angle: {}", structure.angle),
        format!("cover: {}", structure.cover.brief),
        String::new(),
    ];
    for (index, section) in structure.sections.iter().enumerate() {
        lines.push(format!(
            "{}. {} ({} words)",
            index + 1,
            section.heading,
            section.words
        ));
        lines.push(format!("   purpose: {}", section.purpose));
        lines.extend(section.beats.iter().map(|beat| format!("   - {beat}")));
        lines.push(format!("   image: {}", section.image.brief));
        lines.push(String::new());
    }
    lines.push(format!("closing: {}", structure.closing));
    lines.join("\n")
}

/// The dek, then each section under its heading with the picture right under that.
pub fn assemble_markdown(structure: &Structure, bodies: &[String]) -> String {
    let mut lines = vec![format!("_{}_", structure.dek.trim()), String::new()];
    for (index, section) in structure.sections.iter().enumerate() {
        lines.extend([
            format!("## {}", section.heading.trim()),
            String::new(),
            format!(
                "![{}](image:{})",
                section.image.caption.replace(['[', ']'], ""),
                section.key
            ),
            String::new(),
            strip_section_extras(bodies.get(index).map_or("", String::as_str)),
            String::new(),
        ]);
    }
    lines.join("\n").trim().to_owned()
}

static HEADING_LINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^#{1,6}\s").expect("valid"));
static IMAGE_LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^!\[[^\]]*\]\([^)]*\)\s*$").expect("valid"));

/// A model asked for a body alone sometimes adds the heading or a picture anyway.
pub fn strip_section_extras(body: &str) -> String {
    body.split('\n')
        .filter(|line| !HEADING_LINE.is_match(line.trim()) && !IMAGE_LINE.is_match(line.trim()))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned()
}

/// The dek, when the first line is the italic one [`assemble_markdown`] put there.
pub fn dek_of(markdown: &str) -> Option<String> {
    let first = markdown
        .split('\n')
        .map(str::trim)
        .find(|line| !line.is_empty())?;
    let inner = first.strip_prefix('_')?.strip_suffix('_')?;
    (!inner.is_empty()).then(|| inner.trim().to_owned())
}

/// The markdown without that line.
pub fn without_dek(markdown: &str) -> String {
    let lines: Vec<&str> = markdown.split('\n').collect();
    let Some(at) = lines.iter().position(|line| !line.trim().is_empty()) else {
        return markdown.to_owned();
    };
    if dek_of(lines[at]).is_none() {
        return markdown.to_owned();
    }
    lines[at + 1..].join("\n").trim().to_owned()
}

static IMAGE_ANYWHERE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"!\[[^\]]*\]\([^)]*\)").expect("valid"));

pub fn word_count(markdown: &str) -> usize {
    IMAGE_ANYWHERE
        .replace_all(markdown, "")
        .split_whitespace()
        .filter(|word| word.chars().any(|c| c.is_ascii_alphanumeric()))
        .count()
}

static URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)https?://\S+|\bwww\.\S+").expect("valid"));
static HANDLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(^|[^a-z0-9_])@[a-z0-9_]{2,}").expect("valid"));
static HASHTAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(^|\s)#[a-z0-9_]+").expect("valid"));
static TRAILING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)[ \t]+$").expect("valid"));
static DOUBLE_SPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r" {2,}").expect("valid"));

/// What the style pass did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Styled {
    pub text: String,
    pub fixes: Vec<String>,
    /// Lines that talk money: not cut, reported, because an article that
    /// talks money is not this article.
    pub priced: Vec<String>,
}

/// No urls, no handles, no hashtags. `$TINY` stays.
pub fn article_style(markdown: &str) -> Styled {
    let mut fixes = Vec::new();
    let mut text = markdown.to_owned();
    let urls = URL.find_iter(&text).count();
    if urls > 0 {
        text = URL.replace_all(&text, "").into_owned();
        fixes.push(format!("cut {urls} url(s)"));
    }
    let handles = HANDLE.find_iter(&text).count();
    if handles > 0 {
        text = HANDLE.replace_all(&text, "$1").into_owned();
        fixes.push(format!("cut {handles} handle(s)"));
    }
    let hashtags = HASHTAG.find_iter(&text).count();
    if hashtags > 0 {
        text = HASHTAG.replace_all(&text, "$1").into_owned();
        fixes.push(format!("cut {hashtags} hashtag(s)"));
    }
    text = TRAILING.replace_all(&text, "").into_owned();
    text = DOUBLE_SPACE.replace_all(&text, " ").into_owned();
    let priced = text
        .split('\n')
        .filter(|line| PRICE.is_match(line))
        .map(|line| crate::backrooms::text::head(line.trim(), 120))
        .collect();
    Styled {
        text,
        fixes,
        priced,
    }
}

#[cfg(test)]
#[path = "stages_tests.rs"]
pub(crate) mod tests;
