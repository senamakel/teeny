//! A run on disk, twice: the whole record as JSON, for replay and for the
//! cut later, and a markdown transcript for reading. Nothing here is a
//! post; the directory is a notebook.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::Serialize;

use super::BackroomsRun;
use super::distill::Thought;
use super::evaluate::{Craziness, RunStats, js_number, render_scores, render_turn};
use super::gm::GmPost;

/// What a run carries besides its transcript.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct SavedExtras {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats: Option<RunStats>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub judge: Option<Craziness>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thought: Option<Thought>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gm: Option<GmPost>,
}

/// Where a run was written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Saved {
    pub json: PathBuf,
    pub markdown: PathBuf,
}

/// Writes `<id>.json` and `<id>.md` under `dir`.
pub fn save(dir: &Path, run: &BackroomsRun, extras: &SavedExtras) -> std::io::Result<Saved> {
    std::fs::create_dir_all(dir)?;
    let json = dir.join(format!("{}.json", run.id));
    let markdown = dir.join(format!("{}.md", run.id));
    let mut record = serde_json::to_value(run).map_err(std::io::Error::other)?;
    if let (Some(record), serde_json::Value::Object(more)) = (
        record.as_object_mut(),
        serde_json::to_value(extras).map_err(std::io::Error::other)?,
    ) {
        record.extend(more);
    }
    std::fs::write(
        &json,
        serde_json::to_string_pretty(&record).map_err(std::io::Error::other)?,
    )?;
    std::fs::write(&markdown, render_markdown(run, extras))?;
    Ok(Saved { json, markdown })
}

pub fn render_markdown(run: &BackroomsRun, extras: &SavedExtras) -> String {
    let agents = run
        .agents
        .iter()
        .map(|agent| {
            format!(
                "{} @ {} ({} tok; {})",
                agent.agent.id,
                js_number(
                    run.settings
                        .temps
                        .get(&agent.agent.id)
                        .copied()
                        .unwrap_or(agent.agent.temperature)
                ),
                agent.tokens,
                agent
                    .blocks
                    .iter()
                    .map(|block| serde_json::to_value(block.kind)
                        .ok()
                        .and_then(|v| v.as_str().map(str::to_owned))
                        .unwrap_or_default())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
        .collect::<Vec<_>>()
        .join(" · ");
    let seed_title = run
        .seed
        .title
        .as_deref()
        .map(|title| format!("**{title}** — "))
        .unwrap_or_default();
    let mut lines = vec![
        format!("# backrooms {}", run.id),
        String::new(),
        format!(
            "seed ({}): {seed_title}{}",
            run.seed.provenance(),
            run.seed.text
        ),
        String::new(),
        format!("agents: {agents}"),
        format!(
            "settings: rng {}, {} turns, budget {}, drift every {}, {} tool calls/turn",
            run.settings.rng,
            run.settings.turns,
            run.settings.budget,
            run.settings.drift_every,
            run.settings.max_tool_calls
        ),
    ];
    for agent in &run.agents {
        for problem in &agent.problems {
            lines.push(format!("problem ({}): {problem}", agent.agent.id));
        }
    }
    lines.push(String::new());
    if let Some(thought) = &extras.thought {
        lines.extend([
            "## the thought".into(),
            String::new(),
            format!("*{}*", thought.premise),
            String::new(),
        ]);
        if !thought.wire.is_empty() {
            lines.extend([format!("the wire: {}", thought.wire), String::new()]);
        }
        lines.extend([
            thought.thought.clone(),
            String::new(),
            "**as a post**".into(),
            String::new(),
        ]);
        lines.extend(thought.post.split('\n').map(|line| format!("> {line}")));
        lines.push(String::new());
        lines.push(if thought.objects.is_empty() {
            String::new()
        } else {
            format!("objects: {}", thought.objects.join(" · "))
        });
        lines.push(String::new());
    }
    if let Some(gm) = &extras.gm {
        lines.extend(["## the good morning".into(), String::new()]);
        lines.extend(gm.text.split('\n').map(|line| format!("> {line}")));
        lines.extend([
            String::new(),
            if gm.wire.is_empty() {
                "the wire: (not in the post)".into()
            } else {
                format!("the wire: {}", gm.wire)
            },
            if gm.continues.is_empty() {
                String::new()
            } else {
                format!("continues: {}", gm.continues)
            },
            gm.reasons.clone(),
            String::new(),
        ]);
    }
    lines.extend(["## transcript".into(), String::new()]);
    lines.extend(
        run.turns
            .iter()
            .enumerate()
            .map(|(i, turn)| render_turn(turn, i)),
    );
    if let Some(stats) = &extras.stats {
        lines.extend([
            "## stats".into(),
            String::new(),
            "```json".into(),
            serde_json::to_string_pretty(stats).unwrap_or_default(),
            "```".into(),
            String::new(),
        ]);
    }
    if let Some(judge) = &extras.judge {
        lines.extend([
            "## judge".into(),
            String::new(),
            render_scores(judge),
            String::new(),
            judge.reasons.clone(),
            String::new(),
        ]);
        lines.extend(judge.quotables.iter().map(|line| format!("> {line}")));
        lines.extend([
            String::new(),
            if judge.dead_spots.is_empty() {
                "no dead spots".into()
            } else {
                format!(
                    "dead spots: {}",
                    judge
                        .dead_spots
                        .iter()
                        .map(u32::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            },
            String::new(),
            format!("verdict: {}", judge.verdict),
            String::new(),
        ]);
    }
    lines.join("\n")
}

/// `YYYYMMDD-HHMM-<slug>`.
pub fn run_id(now: DateTime<Utc>, slug: &str) -> String {
    let stamp = now.format("%Y%m%d-%H%M").to_string();
    let clean = slugify(slug, 40);
    if clean.is_empty() {
        stamp
    } else {
        format!("{stamp}-{clean}")
    }
}

/// Lowercase, runs of anything else as one `-`, trimmed, at most `max`.
pub fn slugify(text: &str, max: usize) -> String {
    let mut out = String::new();
    let mut dash = false;
    for ch in text.to_lowercase().chars() {
        if ch.is_ascii_lowercase() || ch.is_ascii_digit() {
            if dash && !out.is_empty() {
                out.push('-');
            }
            dash = false;
            out.push(ch);
        } else {
            dash = true;
        }
    }
    out.chars().take(max).collect()
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
