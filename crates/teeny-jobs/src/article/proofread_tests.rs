use std::sync::Mutex;

use async_trait::async_trait;
use serde_json::json;

use super::*;
use crate::backrooms::testing::scratch;

const SKILL: &str = "# Humanizer\n\n### 14. Em dashes\n\nToo many.";
const DRAFT: &str = "_dek_\n\n## the gap\n\n![the door](image:s1)\n\nthe door — it was open — an inch.\n\n## the lock\n\n![the lock](image:s2)\n\nthe lock sat there.";

fn clean() -> serde_json::Value {
    json!({ "verdict": "clean", "findings": [], "voice": "yes" })
}

fn fix_it() -> serde_json::Value {
    json!({ "verdict": "fix", "findings": [{ "section": "s1", "pattern": "14 em dashes", "quote": "the door — it was open — an inch.", "fix": "the door was open an inch." }], "voice": "yes" })
}

/// A reviewer and a fixer sharing one call log.
struct Models {
    reviews: Mutex<Vec<serde_json::Value>>,
    fix: String,
    calls: Mutex<Vec<&'static str>>,
    asked: Mutex<Vec<String>>,
}

struct Reviewer<'a>(&'a Models);
struct Fixer<'a>(&'a Models);

#[async_trait]
impl TextModel for Reviewer<'_> {
    async fn complete(&self, _: Option<&str>, _: &str) -> teeny_media::Result<String> {
        self.0.calls.lock().unwrap().push("review");
        let mut reviews = self.0.reviews.lock().unwrap();
        Ok(if reviews.is_empty() {
            clean()
        } else {
            reviews.remove(0)
        }
        .to_string())
    }
}

#[async_trait]
impl TextModel for Fixer<'_> {
    async fn complete(&self, _: Option<&str>, prompt: &str) -> teeny_media::Result<String> {
        self.0.calls.lock().unwrap().push("fix");
        self.0.asked.lock().unwrap().push(prompt.to_owned());
        Ok(self.0.fix.clone())
    }
}

fn models(reviews: Vec<serde_json::Value>, fix: &str) -> Models {
    Models {
        reviews: Mutex::new(reviews),
        fix: fix.into(),
        calls: Mutex::new(Vec::new()),
        asked: Mutex::new(Vec::new()),
    }
}

async fn run(m: &Models) -> ProofResult {
    proofread(&Reviewer(m), &Fixer(m), SKILL, DRAFT, "s")
        .await
        .unwrap()
}

#[test]
fn reads_with_the_skill_as_the_checklist_and_the_sample_for_the_voice() {
    let ask = proof_prompt(SKILL, DRAFT, "the door was one inch open.");
    assert!(ask.system.contains("### 14. Em dashes"));
    assert!(ask.system.contains("proof-reading, not rewriting"));
    assert!(ask.prompt.contains("the door was one inch open."));
    assert!(ask.prompt.contains(r#""verdict":"clean"|"fix""#));
    let findings: Vec<Finding> = serde_json::from_value(fix_it()["findings"].clone()).unwrap();
    let fix = proof_fix_prompt(SKILL, DRAFT, &findings, "sample");
    assert!(fix.prompt.contains(
        "[s1] 14 em dashes: \"the door — it was open — an inch.\" → the door was open an inch."
    ));
    assert!(fix.system.contains("keep every `## ` heading"));
}

#[tokio::test]
async fn skips_the_fix_when_the_review_is_clean() {
    let m = models(vec![clean()], DRAFT);
    let out = run(&m).await;
    assert!(!out.applied);
    assert_eq!(out.markdown, DRAFT);
    assert_eq!(*m.calls.lock().unwrap(), ["review"]);
}

#[tokio::test]
async fn applies_a_finding_that_quotes_the_text_exactly_by_hand_then_reads_again() {
    let m = models(vec![fix_it(), clean()], DRAFT);
    let out = run(&m).await;
    assert!(out.applied);
    assert_eq!(
        out.markdown,
        DRAFT.replace(
            "the door — it was open — an inch.",
            "the door was open an inch."
        )
    );
    assert_eq!(out.after.unwrap().verdict, Verdict::Clean);
    assert_eq!(*m.calls.lock().unwrap(), ["review", "review"]);
}

#[tokio::test]
async fn asks_the_model_only_about_findings_it_cannot_apply_by_hand() {
    let vague = json!({ "verdict": "fix", "findings": [{ "section": "s2", "pattern": "3 shallow -ing", "quote": "sat there", "fix": "cut the trailing clause" }], "voice": "yes" });
    let fixed = DRAFT.replace("the lock sat there.", "the lock sat.");
    let m = models(vec![vague, clean()], &format!("```markdown\n{fixed}\n```"));
    let out = run(&m).await;
    assert!(out.applied);
    assert_eq!(out.markdown, fixed);
    assert_eq!(*m.calls.lock().unwrap(), ["review", "fix", "review"]);
    assert!(m.asked.lock().unwrap()[0].contains("[s2] 3 shallow -ing"));
}

#[tokio::test]
async fn refuses_a_model_fix_that_changed_the_shape_keeping_what_was_done_by_hand() {
    let vague = json!({ "verdict": "fix", "findings": [{ "section": "s2", "pattern": "3", "quote": "sat there", "fix": "cut it" }], "voice": "yes" });
    for (fix, reason) in [
        (
            "Nothing to fix.".to_owned(),
            "the fix said there was nothing to fix",
        ),
        (
            DRAFT.replace("## the lock", "## the key"),
            "the headings changed",
        ),
        (
            DRAFT.replace("![the lock](image:s2)\n\n", ""),
            "an image placeholder moved or went missing",
        ),
        (
            "_dek_\n\n## the gap\n\n![the door](image:s1)\n\n## the lock\n\n![the lock](image:s2)"
                .to_owned(),
            "of the length",
        ),
    ] {
        let m = models(vec![vague.clone()], &fix);
        let out = run(&m).await;
        assert!(!out.applied);
        assert_eq!(out.markdown, DRAFT);
        assert!(
            out.refused.as_deref().unwrap().contains(reason),
            "{:?}",
            out.refused
        );
    }
    let mut mixed = fix_it();
    mixed["findings"]
        .as_array_mut()
        .unwrap()
        .push(vague["findings"][0].clone());
    let m = models(vec![mixed, clean()], "Nothing to fix.");
    let out = run(&m).await;
    assert!(out.applied);
    assert!(out.markdown.contains("the door was open an inch."));
    assert!(out.refused.unwrap().ends_with("applied by hand"));
    assert_eq!(structurally_same(DRAFT, DRAFT), None);
    assert_eq!(unfence("```md\nx\n```"), "x");
}

#[test]
fn loads_the_skill_without_its_frontmatter() {
    let dir = scratch("humanizer");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("SKILL.md"),
        "---\nname: humanizer\ndescription: x\n---\n\n# Humanizer\n\nbody\n",
    )
    .unwrap();
    assert_eq!(
        load_humanizer(&dir.join("SKILL.md")).unwrap(),
        "# Humanizer\n\nbody"
    );
    assert_eq!(strip_frontmatter("no frontmatter"), "no frontmatter");
    std::fs::remove_dir_all(dir).ok();
}
