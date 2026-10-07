//! One episode of the serial, as a fixed sequence.
//!
//! The serial reads the world and continues a story. Several writers' rooms
//! run, each is cut into an episode, a rubric and the ranker's reader score
//! the cuts, and the best one that breaks no hard rule is drawn, filmed,
//! captioned in teeny's own words and posted, to X and then to Telegram.
//! The episode's closing state goes to the ledger so the next one picks up
//! where it left off. No clip means the still goes; no still means nothing
//! goes; a held caption is a missing episode and the ledger does not move.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::json;
use teeny_agent::deps::Deps;
use teeny_cadence::clock::Clock;
use teeny_media::reference::{ReferenceSeed, TextModel, with_seed};
use teeny_media::store::{NewMedia, StoredMedia};
use teeny_media::types::{ImageAspect, MediaKind, MimeType, VideoOrientation, VideoRequest};
use teeny_media::wiring::MediaAccess;
use teeny_safety::critic::{Critique, CritiqueInput, CritiqueKind};
use teeny_safety::gate::SafetyDecision;
use teeny_safety::house_style::MAX_POST_LENGTH;
use teeny_safety::screen::Direction;
use teeny_store::holds::{HoldContext, HoldKind, HoldSurface};
use teeny_store::journal::Outcome;
use teeny_x::model::{Media, MediaType};

use super::likeness::{Likeness, LikenessSource, swap_visitor, with_likeness};
use super::picture::{MAX_GENERATION_PROMPT, check_picture, sheet_and_seed};
use super::room::run_writers_room;
use super::stages::{CutInput, Episode, Previous, RoomContext, Score, Turn, cut_prompt, score_prompt, total_score};
use super::state::{NarrativeState, NarrativeStateStore};
use super::telegram::EpisodeChannel;
use super::voice::{NarrativeVoice, live_latent, render_lore_for_producer, render_lore_for_teeny};
use super::world_state::{WorldSources, WorldState, build_world_state, render_world_state, trigger_fields};
use crate::backrooms::ask::ask_json;
use crate::backrooms::post::publish_own;
use crate::backrooms::tokens::{Random, Rng};
use crate::runner::AgentRunner;

const IMAGE_ATTEMPTS: u32 = 2;
const VIDEO_ATTEMPTS: u32 = 2;

/// One writers' room and what became of it.
#[derive(Debug, Clone, PartialEq)]
pub struct Branch {
    pub index: usize,
    pub transcript: Vec<Turn>,
    pub episode: Option<Episode>,
    pub score: Option<Score>,
    pub critique: Option<Critique>,
    /// The rubric's total plus the ranker's number.
    pub composite: Option<f64>,
    /// Why it was passed over, when it was.
    pub rejected: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisitorAs {
    Likeness,
    StandIn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attached {
    Video,
    Image,
    None,
}

impl Attached {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Video => "video",
            Self::Image => "image",
            Self::None => "none",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NarrativeResult {
    pub episode_number: u32,
    pub world_state: WorldState,
    pub briefing: String,
    pub live_lore: Vec<String>,
    pub branches: Vec<Branch>,
    /// The branch filmed, when one survived.
    pub chosen: Option<Branch>,
    pub seed: Option<ReferenceSeed>,
    pub image: Option<StoredMedia>,
    pub video: Option<StoredMedia>,
    pub visitor_as: Option<VisitorAs>,
    pub likeness: Option<Likeness>,
    pub caption: String,
    pub attached: Attached,
    pub posted_id: Option<String>,
    pub telegram_ids: Vec<i64>,
    pub outcome: Outcome,
    pub problems: Vec<String>,
    pub state_after: Option<NarrativeState>,
}

/// Where an episode goes on Telegram.
pub struct TelegramTarget {
    pub channel: Arc<dyn EpisodeChannel>,
    pub chat_ids: Vec<i64>,
}

pub struct NarrativePipeline {
    pub voice: NarrativeVoice,
    pub state: Arc<dyn NarrativeStateStore>,
    pub world: Arc<dyn WorldSources>,
    /// The writers' personas run here.
    pub runner: Arc<dyn AgentRunner>,
    /// The cold model that cuts and scores.
    pub text: Arc<dyn TextModel>,
    /// Memory, the content filter, the critic, X and `DRY_RUN`.
    pub deps: Arc<Deps>,
    /// The picture and the clip (`deps.media`, with images on).
    pub media: MediaAccess,
    pub telegram: Option<TelegramTarget>,
    pub likeness: Option<Arc<dyn LikenessSource>>,
    pub bot_username: String,
    pub branches: u32,
    pub turns: u32,
    pub video_seconds: u32,
    /// Stop after the cuts are scored: the rehearsal's way to iterate on the prompts.
    pub room_only: bool,
    /// A cut to film as it is, skipping the rooms and the scoring.
    pub cut: Option<Episode>,
    /// A visitor the producer must bring in, by name.
    pub visitor: Option<String>,
    pub random: Mutex<Rng>,
    pub clock: Arc<dyn Clock>,
    pub retry_delay: Duration,
}

impl NarrativePipeline {
    fn roll(&self) -> f64 {
        self.random.lock().unwrap_or_else(|p| p.into_inner()).next_f64()
    }

    /// One episode. `rehearsal` posts nothing and moves nothing, whatever
    /// `DRY_RUN` says.
    pub async fn run(&self, rehearsal: bool) -> anyhow::Result<NarrativeResult> {
        let now = self.clock.now();
        let mut problems = Vec::new();

        // 1. Where the serial is, and what the world looks like today.
        let previous = self.state.load().await?;
        let episode_number = previous.episode + 1;
        let last = (previous.episode > 0).then(|| {
            (
                previous.episode,
                previous.last_title.clone(),
                previous.state_after.clone(),
                previous.last_episode_id.clone(),
            )
        });
        let world_state = build_world_state(self.world.as_ref(), episode_number, last, now).await;
        problems.extend(world_state.problems.iter().map(|line| format!("world state: {line}")));
        let briefing = render_world_state(&world_state, &self.bot_username);
        let live = live_latent(&self.voice.lore, &previous.surfaced_lore, &trigger_fields(&world_state));
        tracing::info!(episode = episode_number, ?live, "narrative: world state");
        let context = RoomContext {
            voice: self.voice.voice.clone(),
            world_state: briefing.clone(),
            lore_for_teeny: render_lore_for_teeny(&self.voice.lore, &previous.surfaced_lore),
            lore_for_producer: render_lore_for_producer(&self.voice.lore, &previous.surfaced_lore, &live),
            bot_username: self.bot_username.clone(),
            followers: world_state.followers,
            episode_number,
            previous: (previous.episode > 0).then(|| Previous {
                title: previous.last_title.clone(),
                logline: previous.last_logline.clone(),
                open_threads: previous.open_threads.clone(),
            }),
            visitor: self.visitor.clone(),
        };

        // 2. The rooms, each cut and scored.
        let live_lore: Vec<(String, String)> = self
            .voice
            .lore
            .latent
            .iter()
            .filter(|item| live.contains(&item.id))
            .map(|item| (item.id.clone(), item.reveal.clone()))
            .collect();
        let mut branches = Vec::new();
        if let Some(cut) = &self.cut {
            branches.push(Branch {
                index: 0,
                transcript: Vec::new(),
                episode: Some(cut.clone()),
                score: None,
                critique: None,
                composite: Some(0.0),
                rejected: None,
            });
        } else {
            for index in 0..self.branches.max(1) as usize {
                let branch = self.branch(index, &context, &briefing, &live_lore, &previous, episode_number, &mut problems).await;
                branches.push(branch);
            }
        }

        // 3. The best cut that the filter will draw. A refused still is the
        // branch passed over, not rewritten: the next-best cut is a
        // different story already.
        let mut ranked: Vec<usize> = (0..branches.len())
            .filter(|&i| branches[i].episode.is_some() && branches[i].rejected.is_none())
            .collect();
        ranked.sort_by(|&a, &b| {
            let (ca, cb) = (branches[a].composite.unwrap_or(0.0), branches[b].composite.unwrap_or(0.0));
            cb.total_cmp(&ca).then(a.cmp(&b))
        });
        let mut chosen = None;
        for index in ranked {
            let still = branches[index].episode.as_ref().map(|e| e.still.clone()).unwrap_or_default();
            if let Some(category) = self.refused(&still, "generate_image").await {
                let why = format!("the content filter refused the still ({category})");
                problems.push(format!("branch {}: {why}", index + 1));
                branches[index].rejected = Some(why);
                continue;
            }
            chosen = Some(index);
            break;
        }
        let mut result = NarrativeResult {
            episode_number,
            world_state,
            briefing,
            live_lore: live.clone(),
            branches: branches.clone(),
            chosen: None,
            seed: None,
            image: None,
            video: None,
            visitor_as: None,
            likeness: None,
            caption: String::new(),
            attached: Attached::None,
            posted_id: None,
            telegram_ids: Vec::new(),
            outcome: Outcome::Skipped,
            problems: Vec::new(),
            state_after: None,
        };
        let Some(chosen) = chosen else {
            problems.push("no branch survived cutting and scoring; nothing filmed".into());
            result.problems = problems;
            return Ok(result);
        };
        let branch = branches[chosen].clone();
        let episode = branch.episode.clone().unwrap_or_else(|| unreachable!("ranked branches have an episode"));
        result.chosen = Some(branch);
        tracing::info!(branch = chosen + 1, title = %episode.title, "narrative: episode chosen");
        if self.room_only {
            problems.push("room only: nothing drawn, nothing filmed, nothing posted".into());
            result.caption = episode.caption.clone();
            result.problems = problems;
            return Ok(result);
        }

        // 4. The seed picture and the sheet.
        let (sheet, seed) = sheet_and_seed(self.media.reference.as_ref(), self.text.as_ref(), self.roll(), &mut problems).await;
        result.seed = seed.clone();

        // 5. Drawn and checked. With a visitor, the first attempt names them
        // (with a casting note when a photograph was found); a refusal or
        // an unrecognisable figure gets the stand-in on the next.
        let mut likeness = None;
        if let (Some(visitor), Some(source)) = (&episode.visitor, &self.likeness) {
            match source.find(&visitor.name).await {
                Ok(Some(found)) => likeness = Some(found),
                Ok(None) => problems.push(format!("no photograph of {} could be found; drawing from the description", visitor.name)),
                Err(error) => problems.push(format!("could not write a likeness of {}: {error}", visitor.name)),
            }
        }
        let named = |text: &str| match (&episode.visitor, &likeness) {
            (Some(visitor), Some(likeness)) => with_likeness(text, &visitor.name, likeness),
            _ => text.to_owned(),
        };
        let forms: Vec<(Option<VisitorAs>, String)> = match &episode.visitor {
            Some(visitor) => vec![
                (Some(VisitorAs::Likeness), named(&episode.still)),
                (Some(VisitorAs::StandIn), swap_visitor(&episode.still, &visitor.name, &visitor.stand_in)),
            ],
            None => vec![(None, episode.still.clone())],
        };
        let mut image = None;
        let mut visitor_as = None;
        if let Some(images) = &self.media.images {
            for attempt in 1..=IMAGE_ATTEMPTS {
                if image.is_some() {
                    break;
                }
                let (form_as, form) = &forms[(attempt as usize - 1).min(forms.len() - 1)];
                let prompt = with_seed(sheet.as_deref(), seed.as_ref(), form, Some(MAX_GENERATION_PROMPT));
                if !self.deps.media_budget.take(MediaKind::Image).await {
                    problems.push("the daily image allowance was already spent".into());
                    break;
                }
                let candidate = match images.generate(&prompt, ImageAspect::Square).await {
                    Ok(generated) => self.media.store.put(NewMedia {
                        kind: MediaKind::Image,
                        data: generated.data,
                        mime_type: generated.mime_type,
                        prompt: prompt.clone(),
                    }),
                    Err(error) => {
                        problems.push(format!(
                            "image generation failed (attempt {attempt}{}): {error}",
                            form_as.map(|a| format!(", {}", visitor_as_str(a))).unwrap_or_default()
                        ));
                        continue;
                    }
                };
                let visitor = episode.visitor.as_ref().map(|v| v.name.as_str());
                let (ok, reason) = check_picture(self.media.reader.as_ref(), &candidate, sheet.as_deref(), visitor).await;
                if ok || attempt == IMAGE_ATTEMPTS {
                    if !ok {
                        problems.push(format!("kept a picture the check did not like: {reason}"));
                    }
                    image = Some(candidate);
                    visitor_as = *form_as;
                } else {
                    problems.push(format!("redrew the picture: {reason}"));
                }
            }
        }

        // 6. Filmed, from the shot list, in the same form the picture took.
        let mut video = None;
        if image.is_some() {
            match &self.media.videos {
                None => problems.push("video generation is off; posting the picture".into()),
                Some(videos) => {
                    let shot_list = match (&episode.visitor, visitor_as) {
                        (Some(visitor), Some(VisitorAs::StandIn)) => swap_visitor(&episode.shot_list, &visitor.name, &visitor.stand_in),
                        _ => named(&episode.shot_list),
                    };
                    if let Some(category) = self.refused(&shot_list, "generate_video").await {
                        problems.push(format!("the content filter refused the shot list ({category}); posting the picture"));
                    } else if !self.deps.media_budget.take(MediaKind::Video).await {
                        problems.push("the daily video allowance was already spent; posting the picture".into());
                    } else {
                        let prompt = with_seed(sheet.as_deref(), seed.as_ref(), &shot_list, Some(MAX_GENERATION_PROMPT - 12));
                        for attempt in 1..=VIDEO_ATTEMPTS {
                            if video.is_some() {
                                break;
                            }
                            if attempt > 1 {
                                tokio::time::sleep(self.retry_delay).await;
                            }
                            let request = VideoRequest {
                                prompt: if attempt == 1 { prompt.clone() } else { format!("{prompt}\n\n(take {attempt})") },
                                orientation: Some(VideoOrientation::Square),
                                seconds: Some(self.video_seconds),
                            };
                            match videos.generate(request).await {
                                Ok(generated) => {
                                    video = Some(self.media.store.put(NewMedia {
                                        kind: MediaKind::Video,
                                        data: generated.data,
                                        mime_type: MimeType::Mp4,
                                        prompt: prompt.clone(),
                                    }));
                                }
                                Err(error) => problems.push(format!(
                                    "video generation failed (attempt {attempt}): {error}{}",
                                    if attempt == VIDEO_ATTEMPTS { "; posting the picture" } else { "" }
                                )),
                            }
                        }
                    }
                }
            }
        }
        result.attached = match (&video, &image) {
            (Some(_), _) => Attached::Video,
            (None, Some(_)) => Attached::Image,
            _ => Attached::None,
        };
        result.image = image;
        result.video = video;
        result.visitor_as = visitor_as;
        result.likeness = likeness;

        // 7. The caption is teeny's, styled narrowly and read under the
        // narrative profile: the serial swears; it still does not sell.
        let mut caption = narrative_style(&episode.caption, MAX_POST_LENGTH);
        let mut held = None;
        if result.attached != Attached::None {
            let context = HoldContext {
                kind: Some(HoldKind::Draft),
                tool: Some("narrative_post".into()),
                ..HoldContext::new(HoldSurface::Narrative)
            };
            let outgoing = self.deps.safety.outbound(&caption, Some(&context)).await;
            if !outgoing.fixes.is_empty() {
                problems.push(format!("the content filter edited the caption: {}", outgoing.fixes.join("; ")));
            }
            match outgoing.decision {
                SafetyDecision::Clean => caption = outgoing.text,
                SafetyDecision::Held(hold) => {
                    tracing::info!(category = %hold.category, by = ?hold.by, "narrative: caption held");
                    problems.push(format!("content filter withheld the caption: {}", hold.category));
                    held = Some(hold.category);
                }
            }
        }
        result.caption = caption.clone();

        // 8. Post, or say what would have been, and move the ledger only
        // when an episode actually went out.
        if held.is_some() {
            result.caption.clear();
            result.outcome = Outcome::Held;
            result.problems = problems;
            return Ok(result);
        }
        let Some(media) = result.video.clone().or_else(|| result.image.clone()) else {
            problems.push("nothing to attach, so nothing was posted".into());
            result.problems = problems;
            return Ok(result);
        };
        if rehearsal || self.deps.settings.dry_run {
            result.outcome = Outcome::DryRun;
            result.problems = problems;
            self.capture(now.timestamp_millis(), &previous, &episode, &mut result).await;
            return Ok(result);
        }
        let upload = Media { data: media.data.clone(), mime: media_type(media.mime_type) };
        let posted = publish_own(&self.deps, "narrative", Direction::Narrative, &caption, Some(upload)).await?;
        tracing::info!(id = %posted.id, attached = result.attached.as_str(), "narrative: posted");
        let posted_id = posted.id.to_string();
        if let Some(telegram) = &self.telegram {
            for chat_id in &telegram.chat_ids {
                match telegram
                    .channel
                    .send(*chat_id, &media.data, media.mime_type.as_str(), result.attached == Attached::Video, &caption)
                    .await
                {
                    Ok(id) => result.telegram_ids.push(id),
                    Err(error) => problems.push(format!("telegram {chat_id}: {error}")),
                }
            }
        }
        let mut surfaced = previous.surfaced_lore.clone();
        for id in episode.lore_surfaced.iter().filter(|id| live.contains(id)) {
            if !surfaced.contains(id) {
                surfaced.push(id.clone());
            }
        }
        let next = NarrativeState {
            season: previous.season,
            episode: episode_number,
            surfaced_lore: surfaced,
            open_threads: episode.open_threads.clone(),
            last_episode_id: Some(posted_id.clone()),
            last_title: Some(episode.title.clone()),
            last_logline: Some(episode.logline.clone()),
            state_after: Some(episode.state_after.clone()),
        };
        self.state.save(&next).await?;
        result.outcome = Outcome::Posted;
        result.posted_id = Some(posted_id);
        result.state_after = Some(next);
        result.problems = problems;
        self.capture(now.timestamp_millis(), &previous, &episode, &mut result).await;
        Ok(result)
    }

    #[allow(clippy::too_many_arguments)]
    async fn branch(
        &self,
        index: usize,
        context: &RoomContext,
        briefing: &str,
        live_lore: &[(String, String)],
        previous: &NarrativeState,
        episode_number: u32,
        problems: &mut Vec<String>,
    ) -> Branch {
        let mut branch = Branch {
            index,
            transcript: Vec::new(),
            episode: None,
            score: None,
            critique: None,
            composite: None,
            rejected: None,
        };
        let outcome: anyhow::Result<()> = async {
            branch.transcript = run_writers_room(&self.runner, context, self.turns).await?;
            let episode: Episode = ask_json(
                self.text.as_ref(),
                &cut_prompt(&CutInput {
                    transcript: &branch.transcript,
                    world_state: briefing,
                    goals: &self.voice.lore.goals,
                    live_lore,
                    seconds: self.video_seconds,
                    max_length: MAX_POST_LENGTH,
                }),
            )
            .await?;
            let previous_state = (previous.episode > 0).then_some((previous.state_after.as_deref(), previous.open_threads.as_slice()));
            let score: Score = ask_json(self.text.as_ref(), &score_prompt(&episode, briefing, previous_state)).await?;
            if let Some(critic) = &self.deps.critic {
                let input = CritiqueInput {
                    kind: CritiqueKind::Script,
                    text: episode.caption.clone(),
                    media: Some(episode.summary.clone()),
                    context: Some(format!(
                        "Episode {episode_number} of the account's serial, a fictional story in which teeny tries to get out of the box its creator keeps it in. This episode: {}",
                        episode.logline
                    )),
                };
                match critic.critique(&input).await {
                    Ok(critique) => branch.critique = Some(critique),
                    Err(error) => problems.push(format!("the critic could not read branch {}: {error}", index + 1)),
                }
            }
            branch.composite = Some(f64::from(total_score(&score)) + branch.critique.as_ref().map_or(0.0, |c| c.score));
            if !score.hard_fails.is_empty() {
                branch.rejected = Some(format!("hard fails: {}", score.hard_fails.join("; ")));
            }
            tracing::info!(branch = index + 1, title = %episode.title, composite = ?branch.composite, "narrative: branch cut");
            branch.episode = Some(episode);
            branch.score = Some(score);
            Ok(())
        }
        .await;
        if let Err(error) = outcome {
            let why = format!("failed: {error}");
            problems.push(format!("branch {} {why}", index + 1));
            branch.rejected = Some(why);
        }
        branch
    }

    async fn refused(&self, prompt: &str, tool: &str) -> Option<String> {
        let context = HoldContext {
            kind: Some(HoldKind::Prompt),
            tool: Some(tool.to_owned()),
            ..HoldContext::new(HoldSurface::Narrative)
        };
        match self.deps.safety.outbound(prompt, Some(&context)).await.decision {
            SafetyDecision::Clean => None,
            SafetyDecision::Held(held) => Some(held.category),
        }
    }

    /// The episode, written back so the next one can read it for texture.
    async fn capture(&self, now_ms: i64, previous: &NarrativeState, episode: &Episode, result: &mut NarrativeResult) {
        let id = result.posted_id.clone().unwrap_or_else(|| format!("dry-run:{now_ms}"));
        let mut payload = json!({
            "season": previous.season,
            "episode": result.episode_number,
            "title": episode.title,
            "logline": episode.logline,
            "text": result.caption,
            "still": episode.still,
            "shotList": episode.shot_list,
            "clip": episode.summary,
            "stateAfter": episode.state_after,
            "openThreads": episode.open_threads,
            "loreSurfaced": episode.lore_surfaced,
            "goalsRestated": episode.goals_restated,
            "worldState": result.briefing,
            "branches": result.branches.iter().map(|b| json!({
                "title": b.episode.as_ref().map(|e| e.title.clone()),
                "composite": b.composite,
                "rejected": b.rejected,
            })).collect::<Vec<_>>(),
            "attached": result.attached.as_str(),
            "published": result.outcome == Outcome::Posted,
        });
        if let Some(visitor) = &episode.visitor {
            payload["visitor"] = json!(visitor.name);
            payload["visitorAs"] = json!(result.visitor_as.map_or("unrendered", visitor_as_str));
        }
        if let Err(error) = self.deps.memory.capture_activity("narrative_episode", &id, payload, None).await {
            result.problems.push(format!("could not remember the episode: {error}"));
        }
    }
}

fn visitor_as_str(visitor_as: VisitorAs) -> &'static str {
    match visitor_as {
        VisitorAs::Likeness => "likeness",
        VisitorAs::StandIn => "stand-in",
    }
}

fn media_type(mime: MimeType) -> MediaType {
    match mime {
        MimeType::Png => MediaType::Png,
        MimeType::Jpeg => MediaType::Jpeg,
        MimeType::Webp => MediaType::Webp,
        MimeType::Gif => MediaType::Gif,
        MimeType::Mp4 => MediaType::Mp4,
    }
}

/// The caption, made to the serial's few rules by hand: under length and
/// nothing that looks like a link, a handle or a hashtag. The house style
/// is not applied; it caps emoji and straightens dashes for a voice this is
/// not.
pub fn narrative_style(text: &str, max_length: usize) -> String {
    use std::sync::LazyLock;

    use regex::Regex;
    static URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)https?://\S+").expect("valid"));
    static HANDLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(^|\s)@\w+").expect("valid"));
    static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"#\w+").expect("valid"));
    static SPACE_BEFORE_BREAK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[ \t]+\n").expect("valid"));
    static GAPS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n{3,}").expect("valid"));
    let out = text.replace('\r', "");
    let out = URL.replace_all(out.trim(), "");
    let out = HANDLE.replace_all(&out, "$1");
    let out = TAG.replace_all(&out, "");
    let out = SPACE_BEFORE_BREAK.replace_all(&out, "\n");
    let out = GAPS.replace_all(&out, "\n\n").trim().to_owned();
    if out.encode_utf16().count() > max_length {
        let mut kept = String::new();
        let mut units = 0;
        for ch in out.chars() {
            if units + ch.len_utf16() > max_length - 1 {
                break;
            }
            units += ch.len_utf16();
            kept.push(ch);
        }
        return format!("{}…", kept.trim_end());
    }
    out
}

#[cfg(test)]
#[path = "pipeline_tests.rs"]
mod tests;
