//! The good-morning post, as a fixed sequence rather than an agent: read the
//! news and the timeline, choose one story that is not political and not a
//! repeat, draw Tiny inside it, script and film a clip, caption it, gate it,
//! post it, remember it. A port of `legacy/src/morning/pipeline.ts`.
//!
//! A stage that fails degrades where it can: no clip means the picture goes
//! out; no picture means nothing goes out, because the visual is the post.
//! The problems list says what happened. The backrooms morning
//! (`MORNING_BACKROOMS`) is not wired here: the room lives with the
//! backrooms jobs.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use teeny_media::reference::{ReferenceSeed, ReferenceSheet, TextModel, with_seed};
use teeny_media::store::{MediaBudget, MediaStore, NewMedia, StoredMedia};
use teeny_media::types::{
    ImageAspect, ImageGenerator, ImageInput, ImageReader, MediaKind, MimeType, VideoGenerator,
    VideoOrientation, VideoRequest,
};
use teeny_memory::Memory;
use teeny_safety::critic::{Critic, Critique, CritiqueInput, CritiqueKind, CritiqueVerdict, render_critique};
use teeny_safety::gate::{SafetyDecision, SafetyGate};
use teeny_safety::house_style::{
    HouseStyleOptions, apply_house_style, asks_the_reader, count_emoji, without_the_ask,
};
use teeny_store::holds::{HoldContext, HoldKind, HoldSurface};
use teeny_store::journal::Outcome;
use teeny_x::MAX_POST_LENGTH;
use teeny_x::model::Trend;

use super::news::{NewsSource, Story, StoryKind, gather_news, render_stories};
use super::post::{Attachment, OwnPost, OwnPoster, PostFailure};
use super::prompts::{
    Ask, Attached, Avoid, FreshnessKind, PickInput, RefusedScene, Rejected, StageContext,
    caption_prompt, check_question, freshness_prompt, pick_prompt, rewrite_with, scene_prompt,
    script_prompt, seed_brief_prompt,
};
use super::stages::{
    Caption, Check, Freshness, Pick, Scene, Script, SeedBrief, Shape, Verdict, ask_json,
    extract_json, js_len, js_prefix,
};
use super::timeline::{TimelineQuery, TimelineSource, gather_timeline, render_trends};

/// What a generator's prompt may be: the ladder's video surface refuses one
/// over 2000 characters.
pub const MAX_GENERATION_PROMPT: usize = 2_000;
const ARTICLE_CHARACTERS: usize = 3_000;
const IMAGE_ATTEMPTS: u32 = 2;
const VIDEO_ATTEMPTS: u32 = 2;
const PICK_ATTEMPTS: u32 = 3;

/// What Tiny looks like: the sheet, and one official picture to borrow from.
#[async_trait]
pub trait Reference: Send + Sync {
    async fn sheet(&self) -> Option<String>;
    async fn seed(&self, random: f64) -> Option<ReferenceSeed>;
}

#[async_trait]
impl Reference for ReferenceSheet {
    async fn sheet(&self) -> Option<String> {
        self.get().await
    }
    async fn seed(&self, random: f64) -> Option<ReferenceSeed> {
        ReferenceSheet::seed(self, random).await
    }
}

/// Whether the post may go out, or only be rehearsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Post through the poster (which honours `DRY_RUN`).
    Live,
    /// Do everything but post; the rehearsal scripts' guard.
    Rehearsal,
}

/// Everything the morning reads and writes through.
pub struct MorningPipeline {
    pub news: Arc<dyn NewsSource>,
    pub timeline: Option<Arc<dyn TimelineSource>>,
    pub timeline_queries: Vec<TimelineQuery>,
    pub timeline_hours: u32,
    pub posts_per_query: u32,
    pub text: Arc<dyn TextModel>,
    pub images: Arc<dyn ImageGenerator>,
    /// Absent: the post carries the picture.
    pub videos: Option<Arc<dyn VideoGenerator>>,
    /// Eyes to check the picture is Tiny; absent, the first draw is trusted.
    pub reader: Option<Arc<dyn ImageReader>>,
    pub reference: Option<Arc<dyn Reference>>,
    pub store: Arc<MediaStore>,
    pub budget: Arc<dyn MediaBudget>,
    pub memory: Arc<dyn Memory>,
    pub poster: Arc<dyn OwnPoster>,
    /// The content filter on the scene, the script and the caption.
    pub safety: Option<Arc<SafetyGate>>,
    /// The ranker's reader on the script and the caption. Advisory.
    pub critic: Option<Arc<dyn Critic>>,
    /// The persona with its skills.
    pub persona: String,
    pub bot_username: String,
    pub topics: Vec<String>,
    pub news_days: u32,
    pub stories_per_topic: u32,
    pub video_seconds: u32,
    pub max_emoji: usize,
    /// Which seed picture, in `[0, 1)`.
    pub random: f64,
    pub now: DateTime<Utc>,
    /// Between two attempts at a clip.
    pub retry_delay: Duration,
}

/// A subject the freshness check read, with what was chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubjectCheck {
    pub freshness: Freshness,
    pub story: u32,
    pub headline: String,
}

/// What the morning did.
#[derive(Debug, Clone)]
pub struct MorningResult {
    pub stories: Vec<Story>,
    /// What the timeline offered, numbered after the news.
    pub posts: Vec<Story>,
    pub trends: Vec<Trend>,
    pub dropped_as_political: usize,
    pub pick: Pick,
    pub story: Option<Story>,
    pub seed: Option<ReferenceSeed>,
    pub scene: Scene,
    pub image: Option<StoredMedia>,
    pub script: Option<Script>,
    pub video: Option<StoredMedia>,
    pub caption: String,
    pub attached: Attached,
    pub posted_id: Option<String>,
    pub outcome: Outcome,
    pub problems: Vec<String>,
    pub script_critique: Option<Critique>,
    pub caption_critique: Option<Critique>,
    pub subject_checks: Vec<SubjectCheck>,
    pub post_check: Option<Freshness>,
    pub history: String,
    pub community: String,
}

impl MorningResult {
    /// The held caption's note, for the journal.
    pub fn held_note(&self) -> String {
        self.problems
            .iter()
            .find(|problem| problem.starts_with("content filter withheld"))
            .cloned()
            .unwrap_or_else(|| "content filter withheld the caption".into())
    }
}

fn context(surface_tool: &str, kind: HoldKind) -> HoldContext {
    HoldContext {
        kind: Some(kind),
        tool: Some(surface_tool.to_owned()),
        ..HoldContext::new(HoldSurface::Morning)
    }
}

fn subject_of(pick: &Pick, story: Option<&Story>) -> String {
    let mut lines = vec![format!("Headline: {}", pick.headline)];
    if !pick.hook.is_empty() {
        lines.push(format!("Hook: {}", pick.hook));
    }
    lines.push(format!("Angle: {}", pick.angle));
    lines.push(match story {
        Some(story) => format!(
            "Source: {}{} ({})",
            story.source,
            story
                .published_at
                .as_deref()
                .map(|at| format!(", {}", js_prefix(at, 10)))
                .unwrap_or_default(),
            story.url
        ),
        None => "Source: our own, no story".into(),
    });
    if let Some(story) = story.filter(|story| story.kind == StoryKind::X) {
        lines.push(format!("The post: {}", story.excerpt));
    }
    lines.join("\n")
}

fn about(pick: &Pick) -> String {
    let mut lines = vec![
        format!("Story: {}", pick.headline),
        format!("Angle: {}", pick.angle),
    ];
    if !pick.hook.is_empty() {
        lines.push(format!("Hook: {}", pick.hook));
    }
    lines.join("\n")
}

/// A verdict read out of a vision answer that was not clean JSON.
fn loose_verdict(answer: &str) -> bool {
    let lower = answer.to_lowercase();
    if let Some(at) = lower.find("\"ok\"") {
        let rest = lower[at + 4..].trim_start();
        if let Some(rest) = rest.strip_prefix(':') {
            let rest = rest.trim_start();
            if rest.starts_with("true") {
                return true;
            }
            if rest.starts_with("false") {
                return false;
            }
        }
    }
    let start = lower.trim_start();
    start.starts_with("yes") || start.starts_with("true")
}

impl MorningPipeline {
    fn stage(&self) -> StageContext {
        StageContext {
            persona: self.persona.clone(),
            bot_username: self.bot_username.clone(),
            today: self.now.format("%Y-%m-%d").to_string(),
        }
    }

    async fn ask<T: Shape>(&self, ask: &Ask) -> anyhow::Result<T> {
        Ok(ask_json(self.text.as_ref(), &ask.system, &ask.prompt).await?)
    }

    async fn recall(&self, query: &str) -> String {
        self.memory.recall(query, None).await.unwrap_or_default()
    }

    /// The filter's reading of a generation prompt: the category it was
    /// refused for, or nothing.
    async fn refused(&self, prompt: &str, tool: &str) -> Option<String> {
        let safety = self.safety.as_ref()?;
        let result = safety
            .outbound(prompt, Some(&context(tool, HoldKind::Prompt)))
            .await;
        match result.decision {
            SafetyDecision::Clean => None,
            SafetyDecision::Held(held) => Some(held.category),
        }
    }

    async fn freshness(
        &self,
        kind: FreshnessKind,
        text: &str,
        problems: &mut Vec<String>,
    ) -> Option<Freshness> {
        let query = match kind {
            FreshnessKind::Subject => format!(
                "Has this account posted about this, or made this point, in the last two weeks? Quote what it said and when:\n{text}"
            ),
            FreshnessKind::Post => format!(
                "Has this account already said this, or made this same joke or point in other words? Quote what it said and when:\n{text}"
            ),
        };
        let checked = async {
            let recalled = self.memory.recall(&query, None).await?;
            self.ask::<Freshness>(&freshness_prompt(&self.stage(), kind, text, &recalled))
                .await
        };
        match checked.await {
            Ok(fresh) => Some(fresh),
            Err(error) => {
                problems.push(format!(
                    "could not check the {} against memory: {error}",
                    kind.as_str()
                ));
                None
            }
        }
    }

    async fn critique(
        &self,
        stage: &str,
        input: CritiqueInput,
        problems: &mut Vec<String>,
    ) -> Option<Critique> {
        let critic = self.critic.as_ref()?;
        match critic.critique(&input).await {
            Ok(read) => Some(read),
            Err(error) => {
                problems.push(format!("the critic could not read the {stage}: {error}"));
                None
            }
        }
    }

    /// The vision ladder's verdict. No eyes, or eyes that fail, pass.
    async fn check(&self, image: &StoredMedia, sheet: Option<&str>) -> (bool, String) {
        let Some(reader) = &self.reader else {
            return (true, "no vision model to check with".into());
        };
        if image.mime_type == MimeType::Mp4 {
            return (true, "no vision model to check with".into());
        }
        let input = ImageInput::Bytes {
            data: image.data.clone(),
            mime_type: image.mime_type,
        };
        match reader.describe(input, Some(&check_question(sheet))).await {
            Ok(answer) => match extract_json(&answer).and_then(|value| Check::read(&value).ok()) {
                Some(check) => (check.ok, js_prefix(&check.reason, 300)),
                None => (loose_verdict(&answer), js_prefix(&answer, 300)),
            },
            Err(error) => (true, format!("could not check: {error}")),
        }
    }

    /// The caption, sent back once when it asks the reader something or is
    /// over the length or the emoji cap, then put through the house style.
    async fn caption(
        &self,
        pick: &Pick,
        attached: Attached,
        community: &str,
        rewrite: Option<(&str, &Critique)>,
        avoid: Option<&Avoid>,
    ) -> anyhow::Result<String> {
        let mut ask = caption_prompt(
            &self.stage(),
            pick,
            attached,
            community,
            self.max_emoji,
            MAX_POST_LENGTH,
            avoid,
        );
        if let Some((draft, critique)) = rewrite {
            ask.prompt = rewrite_with(&ask.prompt, draft, &render_critique(critique));
        }
        let mut text = self.ask::<Caption>(&ask).await?.text;
        if asks_the_reader(&text) {
            let again = Ask {
                system: ask.system.clone(),
                prompt: format!(
                    "{}\n\nYour draft:\n{text}\n\nIt ends by asking the reader something. Rewrite it to end on your take, the joke, or a wish for the day, with no question in it, keeping everything else.",
                    ask.prompt
                ),
            };
            let retry = self.ask::<Caption>(&again).await?.text;
            text = if asks_the_reader(&retry) {
                without_the_ask(&retry)
            } else {
                retry
            };
        }
        if js_len(&text) > MAX_POST_LENGTH || count_emoji(&text) > self.max_emoji {
            let again = Ask {
                system: ask.system.clone(),
                prompt: format!(
                    "{}\n\nYour draft was {} characters with {} emoji:\n{text}\n\nRewrite it under {MAX_POST_LENGTH} characters with at most {} emoji, keeping the three parts.",
                    ask.prompt,
                    js_len(&text),
                    count_emoji(&text),
                    self.max_emoji
                ),
            };
            text = self.ask::<Caption>(&again).await?.text;
        }
        Ok(apply_house_style(
            &text,
            &HouseStyleOptions {
                max_emoji: self.max_emoji,
                max_length: MAX_POST_LENGTH,
                replying_to: None,
            },
        )
        .text)
    }

    /// Runs the morning.
    pub async fn run(&self, mode: Mode) -> anyhow::Result<MorningResult> {
        let mut problems: Vec<String> = Vec::new();

        // 1. What we already said, and who we are saying it to.
        let (history, community) = tokio::join!(
            self.recall(
                "What good-morning posts and news stories has this account published recently? For each: the story, the topic, and how it was framed. Also anything the account has already said about AI, crypto or tech news this week."
            ),
            self.recall(
                "Who are the holders, builders and community members this account talks to? What do they care about, what have they been asking, and what is the mood right now?"
            ),
        );

        // 2. The news, minus politics, and the timeline next to it.
        let ((stories, dropped_news), timeline) = tokio::join!(
            gather_news(
                self.news.as_ref(),
                &self.topics,
                self.news_days,
                self.stories_per_topic
            ),
            async {
                match &self.timeline {
                    Some(source) => {
                        gather_timeline(
                            source.as_ref(),
                            &self.timeline_queries,
                            self.timeline_hours,
                            self.posts_per_query,
                        )
                        .await
                    }
                    None => Default::default(),
                }
            }
        );
        let posts = timeline.posts;
        let trends = timeline.trends;
        let dropped = dropped_news + timeline.dropped;
        tracing::info!(stories = stories.len(), posts = posts.len(), quiet = timeline.quiet, dropped, "morning: news gathered");
        if self.timeline.is_some() && posts.is_empty() {
            problems.push("the timeline gave nothing; chosen from the news alone".into());
        }

        // 3. The seed picture, briefed down to what is particular about it.
        let sheet = match &self.reference {
            Some(reference) => reference.sheet().await,
            None => None,
        };
        let mut seed = match &self.reference {
            Some(reference) => reference.seed(self.random).await,
            None => None,
        };
        if let Some(found) = seed.take() {
            match self.ask::<SeedBrief>(&seed_brief_prompt(&found.look)).await {
                Ok(brief) => {
                    seed = Some(ReferenceSeed {
                        look: brief.brief,
                        ..found
                    });
                }
                Err(error) => problems.push(format!(
                    "could not brief the seed picture: {error}; drawing from the sheet alone"
                )),
            }
        }

        // 4. The subject, checked against memory, and the scene, checked by
        // the filter, before anything is drawn.
        let candidates: Vec<Story> = stories.iter().chain(posts.iter()).cloned().collect();
        let mut subject_checks = Vec::new();
        let mut rejected: Vec<Rejected> = Vec::new();
        let mut chosen: Option<(Pick, Option<Story>, Scene)> = None;
        let mut scene_refused: Option<String> = None;
        for attempt in 1..=PICK_ATTEMPTS {
            let last = attempt == PICK_ATTEMPTS;
            let ask = pick_prompt(
                &self.stage(),
                &PickInput {
                    stories: render_stories(&stories, 1),
                    timeline: render_stories(&posts, stories.len() + 1),
                    trends: render_trends(&trends),
                    history: history.clone(),
                    rejected: rejected.clone(),
                },
            );
            let mut pick: Pick = self.ask(&ask).await?;
            let story = (pick.story > 0)
                .then(|| candidates.get(pick.story as usize - 1).cloned())
                .flatten();
            if pick.story > 0 && story.is_none() {
                problems.push(format!(
                    "the model chose story {} of {}; using its headline alone",
                    pick.story,
                    candidates.len()
                ));
            }
            if story.is_none()
                && let Some(fallback) = &pick.fallback
            {
                pick.angle = format!("{} ({fallback})", pick.angle);
            }
            tracing::info!(attempt, story = pick.story, headline = pick.headline, "morning: story chosen");

            let subject = subject_of(&pick, story.as_ref());
            if let Some(fresh) = self
                .freshness(FreshnessKind::Subject, &subject, &mut problems)
                .await
            {
                subject_checks.push(SubjectCheck {
                    freshness: fresh.clone(),
                    story: pick.story,
                    headline: pick.headline.clone(),
                });
                if fresh.verdict != Verdict::Fresh {
                    if last {
                        problems.push(format!(
                            "the subject was {} on the last try too ({}); going with it",
                            fresh.verdict.as_str(),
                            fresh.reason
                        ));
                    } else {
                        problems.push(format!(
                            "turned down [{}] {}: {}, {}",
                            pick.story,
                            pick.headline,
                            fresh.verdict.as_str(),
                            fresh.reason
                        ));
                        rejected.push(Rejected {
                            story: pick.story,
                            headline: pick.headline.clone(),
                            verdict: fresh.verdict.as_str().into(),
                            reason: fresh.reason.clone(),
                        });
                        chosen = Some((pick, story, empty_scene()));
                        continue;
                    }
                }
            }

            // The article itself, for the scene: a post is its own article.
            let article = match &story {
                Some(story) if story.kind == StoryKind::X => {
                    let mut lines = vec![format!("{}: {}", story.source, story.excerpt)];
                    if let Some(media) = &story.media {
                        lines.push(format!("Their picture: {media}"));
                    }
                    lines.join("\n")
                }
                Some(story) => self.news.read(&story.url, ARTICLE_CHARACTERS).await,
                None => String::new(),
            };
            let look = seed.as_ref().map(|seed| seed.look.as_str());
            let mut scene: Scene = self
                .ask(&scene_prompt(&pick, &article, look, None))
                .await?;
            scene_refused = self.refused(&scene.scene, "generate_image").await;
            if let Some(category) = scene_refused.clone() {
                problems.push(format!(
                    "the content filter refused the scene ({category}); rewriting it"
                ));
                let refused = RefusedScene {
                    category,
                    scene: scene.scene.clone(),
                };
                scene = self
                    .ask(&scene_prompt(&pick, &article, look, Some(&refused)))
                    .await?;
                scene_refused = self.refused(&scene.scene, "generate_image").await;
            }
            match scene_refused.clone() {
                None => {
                    chosen = Some((pick, story, scene));
                    break;
                }
                Some(category) if last => {
                    problems.push(format!(
                        "the content filter refused the scene again ({category}); nothing drawn"
                    ));
                    chosen = Some((pick, story, scene));
                    break;
                }
                Some(category) => {
                    problems.push(format!(
                        "turned down [{}] {}: the scene was refused twice ({category})",
                        pick.story, pick.headline
                    ));
                    rejected.push(Rejected {
                        story: pick.story,
                        headline: pick.headline.clone(),
                        verdict: "refused".into(),
                        reason: format!(
                            "cannot be drawn without {category} content; the picture was refused twice"
                        ),
                    });
                    chosen = Some((pick, story, scene));
                }
            }
        }
        let (pick, story, scene) = chosen.expect("at least one pick was made");

        // 5. Drawn, then looked at; redrawn once when it is not Tiny.
        let mut image: Option<StoredMedia> = None;
        let image_prompt = with_seed(
            sheet.as_deref(),
            seed.as_ref(),
            &scene.scene,
            Some(MAX_GENERATION_PROMPT),
        );
        let mut attempt = 0;
        while attempt < IMAGE_ATTEMPTS && image.is_none() && scene_refused.is_none() {
            attempt += 1;
            if !self.budget.take(MediaKind::Image).await {
                problems.push("the daily image allowance was already spent".into());
                break;
            }
            let candidate = match self.images.generate(&image_prompt, ImageAspect::Square).await {
                Ok(generated) => self.store.put(NewMedia {
                    kind: MediaKind::Image,
                    data: generated.data,
                    mime_type: generated.mime_type,
                    prompt: image_prompt.clone(),
                }),
                Err(error) => {
                    problems.push(format!(
                        "image generation failed (attempt {attempt}): {error}"
                    ));
                    continue;
                }
            };
            let (ok, reason) = self.check(&candidate, sheet.as_deref()).await;
            if ok || attempt == IMAGE_ATTEMPTS {
                if !ok {
                    problems.push(format!("kept a picture the check did not like: {reason}"));
                }
                image = Some(candidate);
            } else {
                problems.push(format!("redrew the picture: {reason}"));
            }
        }

        // 6. The script, then the clip.
        let mut script: Option<Script> = None;
        let mut script_critique = None;
        if image.is_some() {
            let ask = script_prompt(&self.stage(), &pick, &scene, self.video_seconds);
            let written: anyhow::Result<Script> = async {
                let mut written: Script = self.ask(&ask).await?;
                let input = |text: &str| CritiqueInput {
                    kind: CritiqueKind::Script,
                    text: text.to_owned(),
                    media: None,
                    context: Some(about(&pick)),
                };
                if let Some(read) = self.critique("script", input(&written.script), &mut problems).await {
                    let flat = read.verdict != CritiqueVerdict::Worthy;
                    let critique = render_critique(&read);
                    script_critique = Some(read);
                    if flat {
                        written = self
                            .ask(&Ask {
                                system: ask.system.clone(),
                                prompt: rewrite_with(&ask.prompt, &written.script, &critique),
                            })
                            .await?;
                        if let Some(again) = self.critique("script", input(&written.script), &mut problems).await {
                            script_critique = Some(again);
                        }
                    }
                }
                Ok(written)
            }
            .await;
            match written {
                Ok(written) => script = Some(written),
                Err(error) => problems.push(format!("script failed: {error}")),
            }
        }
        let mut video: Option<StoredMedia> = None;
        match (&script, &self.videos) {
            (Some(_), None) => problems.push("video generation is off; posting the picture".into()),
            (Some(script), Some(videos)) => {
                if let Some(category) = self.refused(&script.script, "generate_video").await {
                    problems.push(format!(
                        "the content filter refused the script ({category}); posting the picture"
                    ));
                } else if !self.budget.take(MediaKind::Video).await {
                    problems.push(
                        "the daily video allowance was already spent; posting the picture".into(),
                    );
                } else {
                    let prompt = with_seed(
                        sheet.as_deref(),
                        seed.as_ref(),
                        &script.script,
                        Some(MAX_GENERATION_PROMPT - 12),
                    );
                    for attempt in 1..=VIDEO_ATTEMPTS {
                        if attempt > 1 {
                            tokio::time::sleep(self.retry_delay).await;
                        }
                        let request = VideoRequest {
                            prompt: if attempt == 1 {
                                prompt.clone()
                            } else {
                                format!("{prompt}\n\n(take {attempt})")
                            },
                            orientation: Some(VideoOrientation::Square),
                            seconds: Some(self.video_seconds),
                        };
                        match videos.generate(request).await {
                            Ok(generated) => {
                                video = Some(self.store.put(NewMedia {
                                    kind: MediaKind::Video,
                                    data: generated.data,
                                    mime_type: MimeType::Mp4,
                                    prompt: prompt.clone(),
                                }));
                                break;
                            }
                            Err(error) => problems.push(format!(
                                "video generation failed (attempt {attempt}): {error}{}",
                                if attempt == VIDEO_ATTEMPTS {
                                    "; posting the picture"
                                } else {
                                    ""
                                }
                            )),
                        }
                    }
                }
            }
            (None, _) => {}
        }

        // 7. The caption: checked against memory as a post, then the ranker.
        let attached = if video.is_some() {
            Attached::Video
        } else if image.is_some() {
            Attached::Image
        } else {
            Attached::None
        };
        let mut caption = self.caption(&pick, attached, &community, None, None).await?;
        let mut post_check = None;
        let mut caption_critique = None;
        if attached != Attached::None {
            if let Some(fresh) = self
                .freshness(FreshnessKind::Post, &caption, &mut problems)
                .await
            {
                let repeated = fresh.verdict != Verdict::Fresh;
                post_check = Some(fresh.clone());
                if repeated {
                    problems.push(format!(
                        "the caption repeated what was already said ({}); rewritten",
                        fresh.reason
                    ));
                    let avoid = Avoid {
                        said: fresh.said,
                        reason: fresh.reason,
                    };
                    caption = self
                        .caption(&pick, attached, &community, None, Some(&avoid))
                        .await?;
                    if let Some(again) = self
                        .freshness(FreshnessKind::Post, &caption, &mut problems)
                        .await
                    {
                        post_check = Some(again);
                    }
                }
            }
            let shows = match (&video, &script) {
                (Some(_), Some(script)) => script.summary.clone(),
                _ => scene.alt.clone(),
            };
            let input = |text: &str| CritiqueInput {
                kind: CritiqueKind::Caption,
                text: text.to_owned(),
                media: Some(shows.clone()),
                context: Some(about(&pick)),
            };
            if let Some(read) = self.critique("caption", input(&caption), &mut problems).await {
                let flat = read.verdict != CritiqueVerdict::Worthy;
                caption_critique = Some(read.clone());
                if flat {
                    caption = self
                        .caption(&pick, attached, &community, Some((&caption, &read)), None)
                        .await?;
                    if let Some(again) = self.critique("caption", input(&caption), &mut problems).await {
                        caption_critique = Some(again);
                    }
                }
            }
        }

        // The gate, outbound, on what would be posted; only when there is
        // something to post.
        let mut held = false;
        if attached != Attached::None
            && let Some(safety) = &self.safety
        {
            let outgoing = safety
                .outbound(&caption, Some(&context("morning_post", HoldKind::Draft)))
                .await;
            if !outgoing.fixes.is_empty() {
                problems.push(format!(
                    "the content filter edited the caption: {}",
                    outgoing.fixes.join("; ")
                ));
            }
            match outgoing.decision {
                SafetyDecision::Clean => caption = outgoing.text,
                SafetyDecision::Held(hold) => {
                    held = true;
                    problems.push(format!("content filter withheld the caption: {}", hold.category));
                }
            }
        }

        let mut result = MorningResult {
            stories,
            posts,
            trends,
            dropped_as_political: dropped,
            pick,
            story,
            seed,
            scene,
            image,
            script,
            video,
            caption,
            attached,
            posted_id: None,
            outcome: Outcome::DryRun,
            problems,
            script_critique,
            caption_critique,
            subject_checks,
            post_check,
            history,
            community,
        };

        // 8. Post, or say what would have been posted.
        if held {
            result.caption = String::new();
            result.outcome = Outcome::Held;
            return Ok(result);
        }
        if attached == Attached::None {
            result
                .problems
                .push("nothing to attach, so nothing was posted".into());
            result.outcome = Outcome::Skipped;
            return Ok(result);
        }
        if mode == Mode::Live {
            let media = result.video.as_ref().or(result.image.as_ref()).expect("attached");
            let post = OwnPost {
                kind: "morning".into(),
                text: result.caption.clone(),
                media: Some(Attachment {
                    data: media.data.clone(),
                    mime_type: media.mime_type,
                }),
                quote: None,
            };
            match self.poster.post(post).await {
                Ok(posted) if posted.dry_run => result.outcome = Outcome::DryRun,
                Ok(posted) => {
                    result.outcome = Outcome::Posted;
                    result.caption = posted.text;
                    result.posted_id = Some(posted.id);
                }
                Err(PostFailure::Held(reason)) => {
                    result.problems.push(reason);
                    result.outcome = Outcome::Skipped;
                    return Ok(result);
                }
                Err(PostFailure::Refused(reason)) => {
                    result.problems.push(format!("content filter withheld the caption: {reason}"));
                    result.caption = String::new();
                    result.outcome = Outcome::Held;
                    return Ok(result);
                }
                Err(PostFailure::Failed(error)) => anyhow::bail!(error),
            }
        }
        self.capture(&mut result).await;
        Ok(result)
    }

    /// What went out, written back so tomorrow's pick can see it. Dry runs
    /// are captured too, labelled.
    async fn capture(&self, result: &mut MorningResult) {
        let id = result
            .posted_id
            .clone()
            .unwrap_or_else(|| format!("dry-run:{}", self.now.timestamp_millis()));
        let mut payload = json!({
            "text": result.caption,
            "topic": result.pick.topic,
            "headline": result.pick.headline,
            "angle": result.pick.angle,
        });
        let entries = payload.as_object_mut().expect("an object");
        if !result.pick.hook.is_empty() {
            entries.insert("hook".into(), json!(result.pick.hook));
        }
        if let Some(story) = &result.story {
            entries.insert("source".into(), json!(story.source));
            entries.insert("url".into(), json!(story.url));
            entries.insert(
                "from".into(),
                json!(if story.kind == StoryKind::X { "x" } else { "news" }),
            );
        }
        if let Some(seed) = &result.seed {
            entries.insert("seed".into(), json!(seed.url));
        }
        entries.insert("scene".into(), json!(result.scene.scene));
        entries.insert("storyCue".into(), json!(result.scene.story_cue));
        if let Some(script) = &result.script {
            entries.insert("script".into(), json!(script.script));
            entries.insert("clip".into(), json!(script.summary));
        }
        entries.insert("attached".into(), json!(result.attached.as_str()));
        entries.insert(
            "published".into(),
            Value::Bool(result.outcome == Outcome::Posted),
        );
        if let Err(error) = self
            .memory
            .capture_activity("morning_post", &id, payload, None)
            .await
        {
            result
                .problems
                .push(format!("could not remember the post: {error}"));
        }
    }
}

fn empty_scene() -> Scene {
    Scene {
        concepts: Vec::new(),
        scene: String::new(),
        story_cue: String::new(),
        alt: String::new(),
    }
}

#[cfg(test)]
#[path = "pipeline_tests.rs"]
mod tests;
