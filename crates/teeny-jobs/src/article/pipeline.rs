//! One article, as a fixed sequence. The backrooms find the idea: a room
//! is opened on something from the wire, steered at one of the lore's
//! engines, and read down to a thought. The article is then planned, written
//! one section at a time, read for flow, proof-read with the humanizer skill
//! as the checklist, held to the voice's hard rules, gated per section,
//! critiqued, drawn, and only then published as an X Article
//! (`ARTICLE_PUBLISH`) or saved as a rehearsal. Every stage's output goes to
//! the record that `/articles` serves.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde_json::json;
use teeny_agent::deps::Deps;
use teeny_cadence::cadence::PublishError as CadenceError;
use teeny_cadence::clock::{Clock, iso};
use teeny_core::types::PostId;
use teeny_media::fit::crop_to_aspect;
use teeny_media::reference::{TextModel, with_seed};
use teeny_media::store::{NewMedia, StoredMedia};
use teeny_media::types::{GeneratedImage, ImageAspect, MediaKind, MimeType};
use teeny_media::wiring::MediaAccess;
use teeny_safety::critic::{
    Critique, CritiqueInput, CritiqueKind, CritiqueVerdict, render_critique,
};
use teeny_safety::gate::SafetyDecision;
use teeny_store::holds::{HoldContext, HoldKind, HoldSurface};
use teeny_store::journal::Outcome;
use teeny_x::article::{ArticleDraft, ContentState};
use teeny_x::model::{Media, MediaType};
use teeny_x::{NoopGuard, XPublisher};

use super::content_state::{ImageRef, markdown_to_content_state, record_shape};
use super::proofread::{ProofResult, proofread, structurally_same, unfence};
use super::stages::{
    ArticleMaterial, Flow, Planning, Structure, StructureCheck, article_style, assemble_markdown,
    check_structure_prompt, dek_of, flow_read_prompt, flow_rewrite_prompt, normalise_structure,
    revise_structure_prompt, section_prompt, strip_section_extras, structure_prompt,
    tighten_prompt, word_count,
};
use super::store::{
    ArticleImage, ArticleRecord, ArticleStatus, FileArticleStore, FlowRecord, ProofRecord,
    RoomLine, SectionRecord, free_slug, slug_for,
};
use crate::backrooms::ask::ask_json;
use crate::backrooms::context::pick_engine;
use crate::backrooms::distill::Thought;
use crate::backrooms::evaluate::{Craziness, craziness_total, render_turn};
use crate::backrooms::seeds::SeedFinder;
use crate::backrooms::tokens::{Random, Rng};
use crate::backrooms::{BackroomsRun, RoomRunner, RunOptions, Seed, SeedKind};
use crate::narrative::picture::{MAX_GENERATION_PROMPT, check_picture, sheet_and_seed};
use crate::narrative::state::NarrativeStateStore;
use crate::narrative::voice::{Lore, render_lore_for_teeny};

const IMAGE_ATTEMPTS: u32 = 2;
/// X shows an article's cover wide; the cover is cut to this.
pub const COVER_ASPECT: f64 = 16.0 / 9.0;

/// One room the article could have come from.
#[derive(Debug, Clone, PartialEq)]
pub struct ArticleRoom {
    pub index: usize,
    pub run_id: Option<String>,
    pub total: Option<f64>,
    pub rejected: Option<String>,
    pub saved: Option<String>,
}

/// A room already on disk, to write from instead of opening one.
#[derive(Debug, Clone, PartialEq)]
pub struct SavedRoom {
    pub run: BackroomsRun,
    pub thought: Thought,
    pub judge: Option<Craziness>,
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArticleResult {
    /// `None` when the run stopped after the plan, as asked.
    pub outcome: Outcome,
    pub structure_only: bool,
    pub slug: Option<String>,
    pub engine: Option<String>,
    pub seed: Option<Seed>,
    pub rooms: Vec<ArticleRoom>,
    pub thought: Option<Thought>,
    pub structure: Option<Structure>,
    pub structure_check: Option<StructureCheck>,
    pub sections: Vec<String>,
    pub markdown: String,
    pub word_count: usize,
    pub flow: Option<FlowRecord>,
    pub proof: Option<ProofResult>,
    pub critique: Option<Critique>,
    pub cover: Option<StoredMedia>,
    pub images: Vec<(String, StoredMedia)>,
    pub content_state: Option<ContentState>,
    pub x_article_id: Option<String>,
    pub post_id: Option<String>,
    pub problems: Vec<String>,
}

pub struct ArticlePipeline {
    pub rooms: Arc<dyn RoomRunner>,
    pub seed: Arc<dyn SeedFinder>,
    pub topics: Vec<String>,
    /// LORE.md as written, for the engines, and parsed, for what teeny may say.
    pub lore_text: String,
    pub lore: Lore,
    /// The serial's ledger, read for what has surfaced; never written.
    pub state: Arc<dyn NarrativeStateStore>,
    /// Warm, for the sections; cold, for the plan, the check and the review; the fixer between.
    pub writer: Arc<dyn TextModel>,
    pub editor: Arc<dyn TextModel>,
    pub fixer: Arc<dyn TextModel>,
    pub humanizer: String,
    pub deps: Arc<Deps>,
    pub media: Option<MediaAccess>,
    pub articles: FileArticleStore,
    pub room_count: u32,
    pub sections_max: u32,
    pub words: u32,
    /// `ARTICLE_PUBLISH`. Off, the run is a rehearsal that saves a `dry-run` record.
    pub publish: bool,
    pub structure_only: bool,
    pub no_images: bool,
    pub no_proof: bool,
    pub no_flow: bool,
    pub from: Option<SavedRoom>,
    pub topic: Option<String>,
    pub engine: Option<String>,
    /// Open the room on this text instead of the wire.
    pub seed_text: Option<String>,
    /// An editorial note the plan, the sections and the check are held to.
    pub brief: Option<String>,
    pub random: Mutex<Rng>,
    pub clock: Arc<dyn Clock>,
}

/// The record as it grows, written at every stop so a crash leaves what was done.
struct Recorder<'a> {
    store: &'a FileArticleStore,
    current: ArticleRecord,
}

impl Recorder<'_> {
    async fn save(
        &mut self,
        status: ArticleStatus,
        problems: &[String],
        files: &[(String, Vec<u8>)],
    ) -> anyhow::Result<()> {
        self.current.status = status;
        self.current.problems = problems.to_vec();
        self.store.save(&self.current, files).await
    }
}

impl ArticlePipeline {
    fn roll(&self) -> f64 {
        self.random
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .next_f64()
    }

    /// One article. `rehearsal` publishes nothing, whatever `ARTICLE_PUBLISH` says.
    pub async fn run(&self, rehearsal: bool) -> anyhow::Result<ArticleResult> {
        let now = self.clock.now();
        let mut problems: Vec<String> = Vec::new();
        let mut record = Recorder {
            store: &self.articles,
            current: ArticleRecord::new(iso(now)),
        };

        // 1. The lore, and what tonight is about.
        let surfaced = self.state.load().await?.surfaced_lore;
        let lore = render_lore_for_teeny(&self.lore, &surfaced);
        let engine = match &self.engine {
            Some(engine) => Some(engine.clone()),
            None => self.choose_engine().await,
        };
        record.current.engine = engine.clone();

        // 2. The seed and 3. the rooms, unless one is handed in.
        let mut rooms = Vec::new();
        let seed;
        let chosen: Option<(BackroomsRun, Thought, Option<Craziness>)>;
        if let Some(from) = &self.from {
            seed = Some(from.run.seed.clone());
            rooms.push(ArticleRoom {
                index: 0,
                run_id: Some(from.run.id.clone()),
                total: from.judge.as_ref().map(craziness_total),
                rejected: None,
                saved: from.path.clone(),
            });
            chosen = Some((from.run.clone(), from.thought.clone(), from.judge.clone()));
        } else {
            let found = self.find_seed(engine.as_deref(), &mut problems).await;
            seed = Some(found.clone());
            let mut best: Option<(f64, BackroomsRun, Thought, Option<Craziness>)> = None;
            for index in 0..self.room_count.max(1) as usize {
                let options = RunOptions {
                    score: true,
                    distill: true,
                    engine: engine.clone(),
                    rng: Some((self.roll() * 1_000_000.0) as u32),
                    ..RunOptions::default()
                };
                match self.rooms.open(&found, &options).await {
                    Err(error) => rooms.push(ArticleRoom {
                        index,
                        run_id: None,
                        total: None,
                        rejected: Some(format!("the room failed: {error}")),
                        saved: None,
                    }),
                    Ok(result) => {
                        let total = result.judge.as_ref().map_or(0.0, craziness_total);
                        rooms.push(ArticleRoom {
                            index,
                            run_id: Some(result.run.id.clone()),
                            total: Some(total),
                            rejected: result
                                .thought
                                .is_none()
                                .then(|| "no thought came out".to_owned()),
                            saved: result.saved.as_ref().map(|s| s.json.display().to_string()),
                        });
                        tracing::info!(index, run = %result.run.id, total, "article: room");
                        if let Some(thought) = result.thought
                            && best.as_ref().is_none_or(|(score, ..)| total > *score)
                        {
                            best = Some((total, result.run, thought, result.judge));
                        }
                    }
                }
            }
            chosen = best.map(|(_, run, thought, judge)| (run, thought, judge));
        }
        record.current.seed = seed.clone();
        record.current.rooms = rooms
            .iter()
            .map(|room| RoomLine {
                run_id: room.run_id.clone(),
                total: room.total,
                rejected: room.rejected.clone(),
            })
            .collect();
        let mut result = ArticleResult {
            outcome: Outcome::Skipped,
            structure_only: false,
            slug: None,
            engine: engine.clone(),
            seed,
            rooms: rooms.clone(),
            thought: None,
            structure: None,
            structure_check: None,
            sections: Vec::new(),
            markdown: String::new(),
            word_count: 0,
            flow: None,
            proof: None,
            critique: None,
            cover: None,
            images: Vec::new(),
            content_state: None,
            x_article_id: None,
            post_id: None,
            problems: Vec::new(),
        };
        let Some((run, thought, judge)) = chosen else {
            problems.push("no room produced a thought".into());
            record.save(ArticleStatus::Skipped, &problems, &[]).await?;
            result.slug = Some(record.current.slug.clone());
            result.problems = problems;
            return Ok(result);
        };
        record.current.run_id = Some(run.id.clone());
        record.current.run_path = rooms
            .iter()
            .find(|r| r.run_id.as_deref() == Some(&run.id))
            .and_then(|r| r.saved.clone());
        record.current.thought = Some(thought.clone());
        record.current.judge = judge;
        result.thought = Some(thought.clone());
        let m = ArticleMaterial {
            run,
            thought: thought.clone(),
            engine: engine.clone(),
            brief: self.brief.clone(),
            lore,
        };

        // 4. The plan, checked once before a word of prose exists.
        let planning = Planning {
            sections_max: self.sections_max,
            words: self.words,
        };
        let mut structure = normalise_structure(
            ask_json(self.editor.as_ref(), &structure_prompt(&m, &planning)).await?,
            self.words,
        );
        let mut check = self.check_structure(&structure, &m, &mut problems).await;
        if let Some(first) = check.clone().filter(|c| !c.ok) {
            tracing::info!(problems = ?first.problems, "article: structure revised");
            match ask_json(
                self.editor.as_ref(),
                &revise_structure_prompt(&structure, &first, &m, &planning),
            )
            .await
            {
                Ok(revised) => {
                    structure = normalise_structure(revised, self.words);
                    let again = self.check_structure(&structure, &m, &mut problems).await;
                    if let Some(again) = again.as_ref().filter(|c| !c.ok) {
                        problems.push(format!(
                            "the plan still has problems after a revision: {}",
                            again.problems.join("; ")
                        ));
                    }
                    check = again.or(check);
                }
                Err(error) => problems.push(format!("could not revise the plan: {error}")),
            }
        }
        record.current.title = structure.title.clone();
        record.current.dek = structure.dek.clone();
        record.current.structure = Some(structure.clone());
        record.current.structure_check = check.clone();
        record.current.slug = free_slug(&self.articles, &slug_for(now, &structure.title)).await?;
        result.slug = Some(record.current.slug.clone());
        result.structure = Some(structure.clone());
        result.structure_check = check.clone();
        if self.structure_only {
            record
                .save(ArticleStatus::Structure, &problems, &[])
                .await?;
            result.outcome = Outcome::DryRun;
            result.structure_only = true;
            result.problems = problems;
            return Ok(result);
        }

        // 5. The sections, in order, each with the earlier ones in view.
        let mut bodies: Vec<String> = Vec::new();
        for index in 0..structure.sections.len() {
            let ask = section_prompt(&structure, index, &bodies, &m);
            let body = unfence(&self.writer.complete(Some(&ask.system), &ask.prompt).await?);
            anyhow::ensure!(
                !body.trim().is_empty(),
                "section {} came back empty",
                index + 1
            );
            bodies.push(strip_section_extras(&body));
        }

        // 6. Cut down when it ran long.
        let mut total: usize = bodies.iter().map(|b| word_count(b)).sum();
        if total as f64 > f64::from(self.words) * 1.25 {
            let mut order: Vec<(usize, i64)> = structure
                .sections
                .iter()
                .enumerate()
                .map(|(i, s)| (i, word_count(&bodies[i]) as i64 - i64::from(s.words)))
                .collect();
            order.sort_by_key(|item| std::cmp::Reverse(item.1));
            for (index, over) in order {
                if over <= 0 || total as f64 <= f64::from(self.words) * 1.1 {
                    break;
                }
                let ask = tighten_prompt(
                    &structure,
                    index,
                    &bodies[index],
                    structure.sections[index].words,
                );
                match self.writer.complete(Some(&ask.system), &ask.prompt).await {
                    Ok(body) if !unfence(&body).trim().is_empty() => {
                        bodies[index] = strip_section_extras(&unfence(&body))
                    }
                    Ok(_) => {}
                    Err(error) => {
                        problems.push(format!("could not tighten section {}: {error}", index + 1))
                    }
                }
                total = bodies.iter().map(|b| word_count(b)).sum();
            }
        }
        let mut markdown = assemble_markdown(&structure, &bodies);
        record.current.sections = structure
            .sections
            .iter()
            .zip(&bodies)
            .map(|(s, body)| SectionRecord {
                key: s.key.clone(),
                heading: s.heading.clone(),
                markdown: body.clone(),
            })
            .collect();
        result.sections = bodies.clone();

        // 7. The flow pass: the first front-to-back read, then one rewrite for continuity.
        if !self.no_flow {
            match ask_json::<Flow>(
                self.editor.as_ref(),
                &flow_read_prompt(&markdown, &structure, &m),
            )
            .await
            {
                Ok(review) => {
                    let mut flow = FlowRecord {
                        review: review.clone(),
                        applied: false,
                        refused: None,
                    };
                    if !review.ok || !review.plan.is_empty() {
                        let ask = flow_rewrite_prompt(&markdown, &review, &structure, &m);
                        match self.writer.complete(Some(&ask.system), &ask.prompt).await {
                            Ok(raw) => {
                                let candidate = unfence(&raw);
                                match structurally_same(&markdown, &candidate) {
                                    Some(why) => {
                                        problems
                                            .push(format!("the flow rewrite was refused: {why}"));
                                        flow.refused = Some(why);
                                    }
                                    None => {
                                        markdown = candidate;
                                        flow.applied = true;
                                    }
                                }
                            }
                            Err(error) => problems.push(format!("the flow pass failed: {error}")),
                        }
                    }
                    record.current.flow = Some(flow.clone());
                    result.flow = Some(flow);
                }
                Err(error) => problems.push(format!("the flow pass failed: {error}")),
            }
        }

        // 8. The proof-reader.
        if !self.no_proof {
            match proofread(
                self.editor.as_ref(),
                self.fixer.as_ref(),
                &self.humanizer,
                &markdown,
                &thought.thought,
            )
            .await
            {
                Ok(proof) => {
                    markdown = proof.markdown.clone();
                    if let Some(why) = &proof.refused {
                        problems.push(format!("the proof-reader's fix was refused: {why}"));
                    }
                    record.current.proof = Some(ProofRecord {
                        review: proof.review.clone(),
                        applied: proof.applied,
                        refused: proof.refused.clone(),
                        after: proof.after.clone(),
                    });
                    result.proof = Some(proof);
                }
                Err(error) => problems.push(format!("the proof-reader failed: {error}")),
            }
        }

        // 9. The voice's hard rules, the safety gate per section, and the critic.
        let styled = article_style(&markdown);
        markdown = styled.text;
        if !styled.fixes.is_empty() {
            problems.push(format!("the style pass {}", styled.fixes.join(", ")));
        }
        if let Some(dek) = dek_of(&markdown).filter(|dek| *dek != structure.dek) {
            structure.dek = dek;
        }
        record.current.markdown = markdown.clone();
        record.current.word_count = word_count(&markdown);
        record.current.dek = structure.dek.clone();
        result.markdown = markdown.clone();
        result.word_count = word_count(&markdown);
        result.structure = Some(structure.clone());
        if !styled.priced.is_empty() {
            problems.push(format!(
                "the article talks money, which it may not: {}",
                styled.priced.join(" | ")
            ));
            record.save(ArticleStatus::Skipped, &problems, &[]).await?;
            result.problems = problems;
            return Ok(result);
        }
        if let Some(category) = self.gate(&structure, &markdown, &mut problems).await {
            tracing::info!(%category, "article: held");
            record.save(ArticleStatus::Held, &problems, &[]).await?;
            result.outcome = Outcome::Held;
            result.problems = problems;
            return Ok(result);
        }
        if let Some(critic) = &self.deps.critic {
            // The critic reads the front of anything long. Cut at a paragraph
            // and say so: a cut mid-sentence was read as the piece ending
            // mid-sentence.
            let front = front_of(&markdown, CRITIC_READ_LENGTH);
            let input = CritiqueInput {
                kind: CritiqueKind::Other,
                context: Some(critic_context(&thought.premise, &front, &markdown)),
                text: front,
                media: None,
            };
            match critic.critique(&input).await {
                Ok(critique) => {
                    record.current.critique = Some(render_critique(&critique));
                    let risky = critique.verdict == CritiqueVerdict::Risky;
                    if risky {
                        problems.push(format!(
                            "the critic read it as risky: {}",
                            if critique.risk_reason.is_empty() {
                                &critique.summary
                            } else {
                                &critique.risk_reason
                            }
                        ));
                    }
                    result.critique = Some(critique);
                    if risky {
                        record.save(ArticleStatus::Skipped, &problems, &[]).await?;
                        result.problems = problems;
                        return Ok(result);
                    }
                }
                Err(error) => problems.push(format!("the critic could not read it: {error}")),
            }
        }

        // 10. Drawn: the cover and one picture per section.
        let mut files: Vec<(String, Vec<u8>)> = Vec::new();
        let mut kept: Vec<ArticleImage> = Vec::new();
        match (&self.media, self.no_images) {
            (_, true) => {}
            (Some(media), false) if media.images.is_some() => {
                let mut wanted = vec![("cover".to_owned(), structure.cover.clone())];
                wanted.extend(
                    structure
                        .sections
                        .iter()
                        .map(|s| (s.key.clone(), s.image.clone())),
                );
                if let Some(left) = self.deps.media_budget.remaining(MediaKind::Image).await
                    && (left as usize) < wanted.len()
                {
                    problems.push(format!("only {left} of the day's image allowance is left; drawing the cover first and stopping when it runs out"));
                }
                let (sheet, look) = sheet_and_seed(
                    media.reference.as_ref(),
                    self.editor.as_ref(),
                    self.roll(),
                    &mut problems,
                )
                .await;
                for (key, brief) in wanted {
                    let prompt = with_seed(
                        sheet.as_deref(),
                        look.as_ref(),
                        &brief.brief,
                        Some(MAX_GENERATION_PROMPT),
                    );
                    let Some(mut drawn) = self
                        .draw(media, &key, &prompt, sheet.as_deref(), &mut problems)
                        .await
                    else {
                        continue;
                    };
                    if drawn.mime_type == MimeType::Mp4 {
                        continue;
                    }
                    if key == "cover" {
                        match crop_to_aspect(
                            &GeneratedImage {
                                data: drawn.data.clone(),
                                mime_type: drawn.mime_type,
                            },
                            COVER_ASPECT,
                        ) {
                            Ok(cut) => {
                                drawn.data = cut.data;
                                drawn.mime_type = cut.mime_type;
                            }
                            Err(error) => {
                                problems.push(format!("could not cut the cover to 16:9: {error}"))
                            }
                        }
                    }
                    let file = format!(
                        "{key}.{}",
                        match drawn.mime_type {
                            MimeType::Jpeg => "jpg",
                            MimeType::Webp => "webp",
                            _ => "png",
                        }
                    );
                    files.push((file.clone(), drawn.data.clone()));
                    let image = ArticleImage {
                        key: key.clone(),
                        file,
                        mime_type: drawn.mime_type.as_str().to_owned(),
                        prompt: drawn.prompt.clone(),
                        caption: brief.caption.clone(),
                        media_id: None,
                    };
                    if key == "cover" {
                        result.cover = Some(drawn);
                        record.current.cover = Some(image);
                    } else {
                        result.images.push((key, drawn));
                        kept.push(image);
                    }
                }
                if result.cover.is_none()
                    && let (Some((_, first)), Some(first_image)) =
                        (result.images.first(), kept.first())
                {
                    problems.push(
                        "no cover could be drawn; the first section picture stands in".into(),
                    );
                    result.cover = Some(first.clone());
                    record.current.cover = Some(ArticleImage {
                        key: "cover".into(),
                        ..first_image.clone()
                    });
                }
                record.current.images = kept.clone();
            }
            _ => problems.push("image generation is off; the article goes without pictures".into()),
        }

        // 11. Saved, then 12. published, or not.
        let live = self.publish && !rehearsal && !self.deps.settings.dry_run;
        if !live {
            let refs: BTreeMap<String, ImageRef> = kept
                .iter()
                .map(|image| {
                    (
                        image.key.clone(),
                        ImageRef {
                            media_id: format!("dry-run:{}", image.key),
                            caption: Some(image.caption.clone()),
                        },
                    )
                })
                .collect();
            let state = markdown_to_content_state(&markdown, &refs);
            record.current.content_state = Some(record_shape(&state));
            record
                .save(ArticleStatus::DryRun, &problems, &files)
                .await?;
            tracing::info!(slug = %record.current.slug, blocks = state.blocks.len(), "article: dry run");
            result.content_state = Some(state);
            result.outcome = Outcome::DryRun;
            result.problems = problems;
            return Ok(result);
        }
        record.save(ArticleStatus::Draft, &problems, &files).await?;
        match self
            .publish_article(&mut record, &structure, &markdown, &mut result, &kept)
            .await
        {
            Ok(()) => {
                record.current.published_at = Some(iso(self.clock.now()));
                record
                    .save(ArticleStatus::Published, &problems, &[])
                    .await?;
                let post_id = result.post_id.clone().unwrap_or_default();
                let payload = json!({
                    "slug": record.current.slug,
                    "title": structure.title,
                    "dek": structure.dek,
                    "engine": engine,
                    "premise": thought.premise,
                    "sections": structure.sections.iter().map(|s| s.heading.clone()).collect::<Vec<_>>(),
                    "published": true,
                });
                if let Err(error) = self
                    .deps
                    .memory
                    .capture_activity("article", &post_id, payload, None)
                    .await
                {
                    problems.push(format!("could not remember the article: {error}"));
                }
                result.outcome = Outcome::Posted;
                result.problems = problems;
                Ok(result)
            }
            Err(error) => {
                record.current.error = Some(error.to_string());
                record.save(ArticleStatus::Failed, &problems, &[]).await?;
                Err(error)
            }
        }
    }

    async fn publish_article(
        &self,
        record: &mut Recorder<'_>,
        structure: &Structure,
        markdown: &str,
        result: &mut ArticleResult,
        kept: &[ArticleImage],
    ) -> anyhow::Result<()> {
        let x = &self.deps.x;
        let upload = async |media: &StoredMedia| -> anyhow::Result<String> {
            let mime = match media.mime_type {
                MimeType::Jpeg => MediaType::Jpeg,
                MimeType::Webp => MediaType::Webp,
                _ => MediaType::Png,
            };
            Ok(x.upload_media(&Media {
                data: media.data.clone(),
                mime,
            })
            .await?
            .media_id)
        };
        let cover_media_id = match &result.cover {
            Some(cover) => Some(upload(cover).await?),
            None => None,
        };
        let mut refs = BTreeMap::new();
        for (key, media) in &result.images {
            let media_id = upload(media).await?;
            let caption = kept
                .iter()
                .find(|i| &i.key == key)
                .map(|i| i.caption.clone());
            refs.insert(key.clone(), ImageRef { media_id, caption });
        }
        if let (Some(cover), Some(id)) = (&mut record.current.cover, &cover_media_id) {
            cover.media_id = Some(id.clone());
        }
        record.current.images = kept
            .iter()
            .map(|image| ArticleImage {
                media_id: refs.get(&image.key).map(|r| r.media_id.clone()),
                ..image.clone()
            })
            .collect();
        let state = markdown_to_content_state(markdown, &refs);
        record.current.content_state = Some(record_shape(&state));
        result.content_state = Some(state.clone());
        let publisher = XPublisher::new(
            Arc::clone(x),
            Arc::new(NoopGuard),
            self.deps.settings.dry_run,
        );
        let draft = ArticleDraft {
            title: structure.title.clone(),
            content_state: state,
            cover_media_id,
        };
        let article_id = Mutex::new(String::new());
        let published = self
            .deps
            .cadence
            .publish_own(
                "article",
                Some(&structure.title),
                async || -> anyhow::Result<PostId> {
                    let published = publisher.publish_article(&draft).await?;
                    *article_id.lock().unwrap_or_else(|p| p.into_inner()) = published.article_id;
                    Ok(published.post_id)
                },
            )
            .await;
        let post_id = match published {
            Ok(post_id) => post_id,
            Err(CadenceError::Held(held)) => anyhow::bail!(held.to_string()),
            Err(CadenceError::Ledger(error)) => anyhow::bail!("cadence: {error}"),
            Err(CadenceError::Publish(error)) => return Err(error),
        };
        let article_id = article_id.into_inner().unwrap_or_default();
        tracing::info!(slug = %record.current.slug, %article_id, %post_id, "article: published");
        record.current.x_article_id = Some(article_id.clone());
        record.current.post_id = Some(post_id.to_string());
        result.x_article_id = Some(article_id);
        result.post_id = Some(post_id.to_string());
        Ok(())
    }

    /// Another engine than the last article's, when the lore has one.
    async fn choose_engine(&self) -> Option<String> {
        let last = self.articles.last_engine().await.ok().flatten();
        let mut random = self.random.lock().unwrap_or_else(|p| p.into_inner());
        for _ in 0..6 {
            let engine = pick_engine(&self.lore_text, &mut *random)?;
            if Some(&engine) != last.as_ref() {
                return Some(engine);
            }
        }
        pick_engine(&self.lore_text, &mut *random)
    }

    /// From the wire on one of the topics, or as a last resort the engine itself.
    async fn find_seed(&self, engine: Option<&str>, problems: &mut Vec<String>) -> Seed {
        if let Some(text) = &self.seed_text {
            return Seed {
                kind: SeedKind::Text,
                title: None,
                text: text.clone(),
                source: Some("the editor".into()),
            };
        }
        let topic = self.topic.clone().unwrap_or_else(|| {
            let at = self
                .random
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .index(self.topics.len());
            self.topics
                .get(at)
                .cloned()
                .unwrap_or_else(|| "ai agents".into())
        });
        match self.seed.find(&topic).await {
            Ok(Some(seed)) => return seed,
            Ok(None) => problems.push(format!("nothing admissible came through the wire for \"{topic}\"; the room opened on the engine alone")),
            Err(error) => problems.push(format!("the wire failed for \"{topic}\": {error}; the room opened on the engine alone")),
        }
        Seed {
            kind: SeedKind::Text,
            title: None,
            text: engine.unwrap_or("the lab at night").to_owned(),
            source: Some("lore".into()),
        }
    }

    async fn check_structure(
        &self,
        structure: &Structure,
        m: &ArticleMaterial,
        problems: &mut Vec<String>,
    ) -> Option<StructureCheck> {
        match ask_json(self.editor.as_ref(), &check_structure_prompt(structure, m)).await {
            Ok(check) => Some(check),
            Err(error) => {
                problems.push(format!("the plan could not be checked: {error}"));
                None
            }
        }
    }

    /// The title and dek once, then each section on its own. The category of the first hold.
    async fn gate(
        &self,
        structure: &Structure,
        markdown: &str,
        problems: &mut Vec<String>,
    ) -> Option<String> {
        let mut pieces = vec![format!("{}\n{}", structure.title, structure.dek)];
        pieces.extend(split_sections(markdown));
        let context = HoldContext {
            kind: Some(HoldKind::Draft),
            tool: Some("article_publish".into()),
            ..HoldContext::new(HoldSurface::Article)
        };
        for piece in pieces {
            let outgoing = self.deps.safety.outbound(&piece, Some(&context)).await;
            if !outgoing.fixes.is_empty() {
                problems.push(format!(
                    "the content filter would edit the article: {}",
                    outgoing.fixes.join("; ")
                ));
            }
            if let SafetyDecision::Held(held) = outgoing.decision {
                problems.push(format!(
                    "content filter withheld the article: {} ({})",
                    held.category, held.reason
                ));
                return Some(held.category);
            }
        }
        None
    }

    async fn draw(
        &self,
        media: &MediaAccess,
        key: &str,
        prompt: &str,
        sheet: Option<&str>,
        problems: &mut Vec<String>,
    ) -> Option<StoredMedia> {
        let images = media.images.as_ref()?;
        let context = HoldContext {
            kind: Some(HoldKind::Prompt),
            tool: Some("generate_image".into()),
            ..HoldContext::new(HoldSurface::Article)
        };
        if let SafetyDecision::Held(held) = self
            .deps
            .safety
            .outbound(prompt, Some(&context))
            .await
            .decision
        {
            problems.push(format!(
                "the content filter refused the {key} picture ({})",
                held.category
            ));
            return None;
        }
        for attempt in 1..=IMAGE_ATTEMPTS {
            if !self.deps.media_budget.take(MediaKind::Image).await {
                problems.push(format!(
                    "the daily image allowance ran out before the {key} picture"
                ));
                return None;
            }
            let candidate = match images.generate(prompt, ImageAspect::Landscape).await {
                Ok(generated) => media.store.put(NewMedia {
                    kind: MediaKind::Image,
                    data: generated.data,
                    mime_type: generated.mime_type,
                    prompt: prompt.to_owned(),
                }),
                Err(error) => {
                    problems.push(format!(
                        "the {key} picture failed (attempt {attempt}): {error}"
                    ));
                    continue;
                }
            };
            let (ok, reason) = check_picture(media.reader.as_ref(), &candidate, sheet, None).await;
            if ok || attempt == IMAGE_ATTEMPTS {
                if !ok {
                    problems.push(format!(
                        "kept a {key} picture the check did not like: {reason}"
                    ));
                }
                return Some(candidate);
            }
            problems.push(format!("redrew the {key} picture: {reason}"));
        }
        None
    }
}

/// How much of the article the critic is shown: the most it reads, in
/// UTF-16 units as the legacy counted.
const CRITIC_READ_LENGTH: usize = 8_000;

/// The article up to `max` UTF-16 units, cut at the last paragraph break
/// that fits (a hard cut when there is none).
pub fn front_of(markdown: &str, max: usize) -> String {
    if teeny_memory::text::units(markdown) <= max {
        return markdown.to_owned();
    }
    let window = teeny_memory::text::cut_at(markdown, max);
    let cut = match window.rfind("\n\n") {
        Some(at) if at > 0 => &markdown[..at],
        _ => window,
    };
    cut.trim_end().to_owned()
}

/// What the critic is told about the piece, and, when it reads only the
/// front, that the stop is not the ending.
pub fn critic_context(premise: &str, front: &str, markdown: &str) -> String {
    let mut context = format!(
        "a long-form article from teeny's serial, published as an x article. the premise: {premise}"
    );
    if front.len() < markdown.len() {
        let words = |text: &str| text.split_whitespace().count();
        context.push_str(&format!(
            "\nthis is the opening {} of its {} words; the piece goes on past where this text stops, so the stop is not its ending.",
            words(front),
            words(markdown)
        ));
    }
    context
}

/// The dek, then each `## ` section with its heading, as separate pieces.
pub fn split_sections(markdown: &str) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    for line in markdown.split('\n') {
        if line.starts_with("## ") && !current.is_empty() {
            pieces.push(current.join("\n").trim().to_owned());
            current.clear();
        }
        current.push(line);
    }
    if !current.is_empty() {
        pieces.push(current.join("\n").trim().to_owned());
    }
    pieces.retain(|piece| !piece.is_empty());
    pieces
}

/// The room's transcript as a reader sees it, for the journal.
pub fn render_room(run: &BackroomsRun) -> String {
    run.turns
        .iter()
        .enumerate()
        .map(|(i, turn)| render_turn(turn, i))
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
#[path = "pipeline_tests.rs"]
mod tests;
