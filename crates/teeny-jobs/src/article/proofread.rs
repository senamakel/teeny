//! The proof-reader: the humanizer skill (`skills/humanizer/SKILL.md`), read
//! twice. First as a reviewer that lists every place the article trips one
//! of the skill's patterns; then, only when there is something to fix, the
//! findings that quote the text exactly are applied by hand and a model is
//! asked only about the rest, its answer held to the draft's shape.

use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use teeny_media::reference::TextModel;

use super::content_state::IMAGE_PLACEHOLDER;
use crate::backrooms::ask::{Ask, Shape, ask_json};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    #[serde(default)]
    pub section: String,
    pub pattern: String,
    pub quote: String,
    pub fix: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Clean,
    Fix,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proof {
    pub verdict: Verdict,
    #[serde(default)]
    pub findings: Vec<Finding>,
    #[serde(default)]
    pub voice: String,
}

impl Shape for Proof {
    fn check(&self) -> Result<(), String> {
        if self.findings.iter().any(|f| {
            f.pattern.trim().is_empty() || f.quote.trim().is_empty() || f.fix.trim().is_empty()
        }) {
            return Err("findings: pattern, quote and fix are required".into());
        }
        Ok(())
    }
}

/// The skill file without its frontmatter, which is for a skill loader.
pub fn load_humanizer(path: &Path) -> std::io::Result<String> {
    Ok(strip_frontmatter(&std::fs::read_to_string(path)?))
}

static FRONTMATTER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^---\n[\s\S]*?\n---\n").expect("valid"));

pub fn strip_frontmatter(text: &str) -> String {
    FRONTMATTER.replace(text, "").trim().to_owned()
}

const PROOF_NOTE: &str = "you are proof-reading, not rewriting. the text below is markdown fiction in the voice of a small robot: lowercase, plain words, short sentences, sincere. that voice is deliberate and is not a finding. the skill above is your checklist. read the whole text against it and list every place it trips one of the numbered patterns: which pattern, the words as written, and the fix. the \"what not to flag\" section applies. verdict is \"clean\" when there is nothing worth changing; \"fix\" otherwise. one line on whether it reads like the writing sample.";

pub fn proof_prompt(skill: &str, markdown: &str, sample: &str) -> Ask {
    Ask::new(
        format!("{skill}\n\n---\n\n{PROOF_NOTE}"),
        [
            "a writing sample, the same writer, for the voice:",
            sample,
            "",
            "the text:",
            "",
            markdown,
            "",
            r#"Shape: {"verdict":"clean"|"fix","findings":[{"section":"s1","pattern":"…","quote":"…","fix":"…"}],"voice":"…"}"#,
        ]
        .join("\n"),
    )
}

const FIX_NOTE: &str = "apply exactly the findings listed, and nothing else. keep every claim, every scene and every detail as written. keep every `## ` heading line and every `![…](image:…)` line exactly where they are, unchanged. keep the lowercase voice. pasted-text mode: return only the rewritten markdown, no preamble, no note on what changed.";

pub fn proof_fix_prompt(skill: &str, markdown: &str, findings: &[Finding], sample: &str) -> Ask {
    let mut prompt = vec![
        "a writing sample, the same writer, for the voice:".to_owned(),
        sample.to_owned(),
        String::new(),
        "the findings:".into(),
    ];
    prompt.extend(findings.iter().map(|f| {
        format!(
            "- [{}] {}: \"{}\" → {}",
            if f.section.is_empty() {
                "?"
            } else {
                &f.section
            },
            f.pattern,
            f.quote,
            f.fix
        )
    }));
    prompt.extend([
        String::new(),
        "the text:".into(),
        String::new(),
        markdown.to_owned(),
    ]);
    Ask::new(format!("{skill}\n\n---\n\n{FIX_NOTE}"), prompt.join("\n"))
}

static NOTHING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(nothing to fix|no changes|the text is fine|no fixes)").expect("valid")
});

/// Why the fixed text is no longer the same article, or `None` when it is.
pub fn structurally_same(before: &str, after: &str) -> Option<String> {
    let trimmed = after.trim();
    if trimmed.is_empty() {
        return Some("the fix came back empty".into());
    }
    if NOTHING.is_match(trimmed) {
        return Some("the fix said there was nothing to fix".into());
    }
    let (b, a) = (
        before.trim().encode_utf16().count(),
        trimmed.encode_utf16().count(),
    );
    if (a as f64) < b as f64 * 0.6 {
        return Some(format!(
            "the fix is {}% of the length",
            ((a as f64 / b as f64) * 100.0).round()
        ));
    }
    let headings = |text: &str| -> Vec<String> {
        text.split('\n')
            .filter(|l| l.starts_with("## "))
            .map(|l| l.trim().to_owned())
            .collect()
    };
    if headings(before) != headings(after) {
        return Some("the headings changed".into());
    }
    let placeholders = |text: &str| -> Vec<String> {
        text.split('\n')
            .filter_map(|l| {
                IMAGE_PLACEHOLDER
                    .captures(l.trim())
                    .map(|c| c[2].to_owned())
            })
            .collect()
    };
    if placeholders(before) != placeholders(after) {
        return Some("an image placeholder moved or went missing".into());
    }
    None
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProofResult {
    pub markdown: String,
    pub review: Proof,
    pub applied: bool,
    pub refused: Option<String>,
    pub after: Option<Proof>,
}

static INSTRUCTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(cut|remove|delete|shorten|rewrite|replace|drop|split|merge)\b")
        .expect("valid")
});

/// The findings that quote the text exactly, applied in place; the rest left.
pub fn apply_findings(
    markdown: &str,
    findings: &[Finding],
) -> (String, Vec<Finding>, Vec<Finding>) {
    let mut text = markdown.to_owned();
    let (mut applied, mut left) = (Vec::new(), Vec::new());
    for finding in findings {
        let quote = finding.quote.trim();
        if !quote.is_empty() && text.contains(quote) && !INSTRUCTION.is_match(finding.fix.trim()) {
            text = text.replace(quote, finding.fix.trim());
            applied.push(finding.clone());
        } else {
            left.push(finding.clone());
        }
    }
    (text, applied, left)
}

pub async fn proofread(
    reviewer: &dyn TextModel,
    fixer: &dyn TextModel,
    skill: &str,
    markdown: &str,
    sample: &str,
) -> anyhow::Result<ProofResult> {
    let review: Proof = ask_json(reviewer, &proof_prompt(skill, markdown, sample)).await?;
    if review.verdict == Verdict::Clean || review.findings.is_empty() {
        return Ok(ProofResult {
            markdown: markdown.to_owned(),
            review,
            applied: false,
            refused: None,
            after: None,
        });
    }
    let (mut fixed, _, left) = apply_findings(markdown, &review.findings);
    let mut refused = None;
    if !left.is_empty() {
        let raw = fixer
            .complete(
                Some(&proof_fix_prompt(skill, &fixed, &left, sample).system),
                &proof_fix_prompt(skill, &fixed, &left, sample).prompt,
            )
            .await?;
        let candidate = unfence(&raw);
        refused = structurally_same(&fixed, &candidate);
        if refused.is_none() {
            fixed = candidate;
        }
    }
    if fixed == markdown {
        return Ok(ProofResult {
            markdown: markdown.to_owned(),
            review,
            applied: false,
            refused,
            after: None,
        });
    }
    // The second read is for the record; the fix stands without it.
    let after = ask_json::<Proof>(reviewer, &proof_prompt(skill, &fixed, sample))
        .await
        .ok();
    Ok(ProofResult {
        markdown: fixed,
        review,
        applied: true,
        refused: refused.map(|why| format!("{why}; the fixes quoted exactly were applied by hand")),
        after,
    })
}

static FENCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*```(?:markdown|md)?\s*\n([\s\S]*?)\n```\s*$").expect("valid")
});

/// A model returning markdown sometimes wraps it in a fence anyway.
pub fn unfence(text: &str) -> String {
    match FENCE.captures(text) {
        Some(c) => c[1].trim().to_owned(),
        None => text.trim().to_owned(),
    }
}

#[cfg(test)]
#[path = "proofread_tests.rs"]
mod tests;
