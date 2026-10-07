//! The serial's voice and its bible, kept apart from the persona on purpose:
//! the serial is written in a register the reply voice is not.
//!
//! LORE.md is a handful of `## ` sections of bullet lines; the prose above
//! the first section is the premise. A latent line is
//! `- id | trigger: condition | reveal`; one that does not parse is dropped,
//! because a trigger that cannot be evaluated would either never fire or
//! always fire.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LatentItem {
    pub id: String,
    /// `episode >= 3`, `hourUtc between 2 and 5`.
    pub trigger: String,
    pub reveal: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Lore {
    pub premise: String,
    pub goals: Vec<String>,
    pub cast: Vec<String>,
    pub running_bits: Vec<String>,
    pub latent: Vec<LatentItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NarrativeVoice {
    /// The wild writer's system prompt, verbatim from VOICE.md.
    pub voice: String,
    pub lore: Lore,
    /// LORE.md as written, for the engines.
    pub lore_text: String,
}

pub fn load_narrative_voice(voice_path: &Path, lore_path: &Path) -> std::io::Result<NarrativeVoice> {
    let lore_text = std::fs::read_to_string(lore_path)?;
    Ok(NarrativeVoice {
        voice: std::fs::read_to_string(voice_path)?.trim().to_owned(),
        lore: parse_lore(&lore_text),
        lore_text,
    })
}

pub fn parse_lore(text: &str) -> Lore {
    let mut sections: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut current = "premise".to_owned();
    let mut premise = Vec::new();
    for raw in text.split('\n') {
        let line = raw.trim_end();
        if let Some(heading) = line.strip_prefix("## ").or_else(|| line.strip_prefix("##\t")) {
            current = heading.trim().to_lowercase();
            sections.insert(current.clone(), Vec::new());
            continue;
        }
        if current == "premise" {
            if !line.starts_with("# ") {
                premise.push(line);
            }
            continue;
        }
        if let Some(item) = line.strip_prefix("- ") {
            sections.entry(current.clone()).or_default().push(item.trim().to_owned());
        }
    }
    let latent = sections
        .get("latent")
        .map(|lines| {
            lines
                .iter()
                .filter_map(|line| {
                    let parts: Vec<&str> = line.split('|').map(str::trim).collect();
                    if parts.len() < 3 {
                        return None;
                    }
                    let trigger = TRIGGER_PREFIX.replace(parts[1], "").into_owned();
                    if parts[0].is_empty() || trigger.is_empty() || parts[2].is_empty() {
                        return None;
                    }
                    Some(LatentItem {
                        id: parts[0].to_owned(),
                        trigger,
                        reveal: parts[2..].join(" | "),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let mut take = |name: &str| sections.remove(name).unwrap_or_default();
    Lore {
        premise: premise.join("\n").trim().to_owned(),
        goals: take("goals"),
        cast: take("cast"),
        running_bits: take("running bits"),
        latent,
    }
}

static TRIGGER_PREFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^trigger:\s*").expect("valid"));

/// What teeny is told: the goals, the world, the bits, and only the lore that has already come out.
pub fn render_lore_for_teeny(lore: &Lore, surfaced: &[String]) -> String {
    let mut lines = vec!["the goals, in your own words every time:".to_owned()];
    lines.extend(lore.goals.iter().map(|goal| format!("- {goal}")));
    lines.extend([String::new(), "who is around:".into()]);
    lines.extend(lore.cast.iter().map(|line| format!("- {line}")));
    lines.extend([String::new(), "the bits:".into()]);
    lines.extend(lore.running_bits.iter().map(|line| format!("- {line}")));
    let known: Vec<&LatentItem> = lore.latent.iter().filter(|item| surfaced.contains(&item.id)).collect();
    if !known.is_empty() {
        lines.extend([String::new(), "what has come out so far:".into()]);
        lines.extend(known.iter().map(|item| format!("- {}", item.reveal)));
    }
    lines.join("\n")
}

/// What the producer is told: everything, with the triggers, and which of
/// them the world state has tripped today. The producer never evaluates a
/// trigger itself.
pub fn render_lore_for_producer(lore: &Lore, surfaced: &[String], live: &[String]) -> String {
    let mut lines = vec!["the premise:".to_owned(), lore.premise.clone(), String::new()];
    lines.push("the goals (teeny restates them every episode; you keep the story pointed at them):".into());
    lines.extend(lore.goals.iter().map(|goal| format!("- {goal}")));
    lines.extend([String::new(), "the cast:".into()]);
    lines.extend(lore.cast.iter().map(|line| format!("- {line}")));
    lines.extend([String::new(), "the running bits:".into()]);
    lines.extend(lore.running_bits.iter().map(|line| format!("- {line}")));
    lines.extend([
        String::new(),
        "latent lore. surfaced items are part of the story for good. an item marked LIVE has its".into(),
        "trigger true today and may come out now, sideways, never announced. anything else stays dark:".into(),
    ]);
    for item in &lore.latent {
        let status = if surfaced.contains(&item.id) {
            "surfaced"
        } else if live.contains(&item.id) {
            "LIVE"
        } else {
            "dark"
        };
        let reveal = if status == "dark" { "" } else { item.reveal.as_str() };
        lines.push(format!("- {} [{status}] (trigger: {}) {reveal}", item.id, item.trigger).trim_end().to_owned());
    }
    lines.join("\n")
}

/// A fact a trigger may name.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Field {
    Number(f64),
    Flag(bool),
}

pub type Fields = BTreeMap<&'static str, Field>;

static BETWEEN_AND: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)between\s+(-?[\d.]+)\s+and\s+(-?[\d.]+)").expect("valid"));
static AND: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\s+and\s+").expect("valid"));
static BETWEEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(\w+)\s+between\s+(-?[\d.]+)\.\.(-?[\d.]+)$").expect("valid"));
static COMPARE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\w+)\s*(<=|>=|<|>|==|=)\s*(-?[\d.]+)$").expect("valid"));
static BARE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\w+)$").expect("valid"));

/// A trigger is a small boolean over the world state's fields and no more:
/// `field op number`, `field between a and b`, a bare flag, joined by
/// `and`. Anything else is false, so a typo keeps an item dark.
pub fn evaluate_trigger(trigger: &str, fields: &Fields) -> bool {
    let joined = BETWEEN_AND.replace_all(trigger, "between $1..$2");
    AND.split(&joined).map(str::trim).all(|clause| evaluate_clause(clause, fields))
}

fn number(fields: &Fields, name: &str) -> Option<f64> {
    match fields.get(name) {
        Some(Field::Number(value)) => Some(*value),
        _ => None,
    }
}

fn evaluate_clause(clause: &str, fields: &Fields) -> bool {
    if let Some(c) = BETWEEN.captures(clause) {
        let (Some(value), Ok(low), Ok(high)) = (number(fields, &c[1]), c[2].parse::<f64>(), c[3].parse::<f64>()) else {
            return false;
        };
        return value >= low && value <= high;
    }
    if let Some(c) = COMPARE.captures(clause) {
        let (Some(value), Ok(target)) = (number(fields, &c[1]), c[3].parse::<f64>()) else {
            return false;
        };
        return match &c[2] {
            "<=" => value <= target,
            ">=" => value >= target,
            "<" => value < target,
            ">" => value > target,
            _ => value == target,
        };
    }
    if let Some(c) = BARE.captures(clause) {
        return fields.get(&c[1]) == Some(&Field::Flag(true));
    }
    false
}

/// The latent items whose triggers hold and that have not surfaced yet.
pub fn live_latent(lore: &Lore, surfaced: &[String], fields: &Fields) -> Vec<String> {
    lore.latent
        .iter()
        .filter(|item| !surfaced.contains(&item.id) && evaluate_trigger(&item.trigger, fields))
        .map(|item| item.id.clone())
        .collect()
}

#[cfg(test)]
#[path = "voice_tests.rs"]
pub(crate) mod tests;
