//! Stand-ins the daily posts' tests share: a text model that answers by the
//! shape it was asked for, X reads from a list, and a poster that records.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use teeny_core::types::{PostId, UserId};
use teeny_media::reference::TextModel;
use teeny_x::XResult;
use teeny_x::cache::XReads;
use teeny_x::model::{OwnPostMetrics, SearchOptions, Trend, TrendOptions, XPost, XUser};

use super::post::{OwnPost, OwnPoster, PostFailure, Posted};

/// A text model answering every prompt with `answer(prompt)`, and keeping
/// what it was asked.
pub struct Staged {
    answer: Box<dyn Fn(&str) -> Result<String, String> + Send + Sync>,
    pub asked: Mutex<Vec<(String, String)>>,
}

impl Staged {
    pub fn new(answer: impl Fn(&str) -> Result<String, String> + Send + Sync + 'static) -> Self {
        Self {
            answer: Box::new(answer),
            asked: Mutex::default(),
        }
    }

    /// Answers from a queue, in order.
    pub fn queue(answers: impl IntoIterator<Item = &'static str>) -> Self {
        let queue: Mutex<VecDeque<String>> =
            Mutex::new(answers.into_iter().map(str::to_owned).collect());
        Self::new(move |_| {
            queue
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| "script exhausted".to_owned())
        })
    }

    /// Every prompt asked, in order.
    pub fn prompts(&self) -> Vec<String> {
        self.asked
            .lock()
            .unwrap()
            .iter()
            .map(|(_, prompt)| prompt.clone())
            .collect()
    }

    /// The prompts that contain `needle`.
    pub fn asked_with(&self, needle: &str) -> Vec<String> {
        self.prompts()
            .into_iter()
            .filter(|prompt| prompt.contains(needle))
            .collect()
    }
}

#[async_trait]
impl TextModel for Staged {
    async fn complete(&self, system: Option<&str>, prompt: &str) -> teeny_media::Result<String> {
        self.asked
            .lock()
            .unwrap()
            .push((system.unwrap_or("").to_owned(), prompt.to_owned()));
        (self.answer)(prompt).map_err(teeny_media::Error::failed)
    }
}

/// X reads answered from fixed lists, with the searches recorded.
#[derive(Default)]
pub struct FakeReads {
    /// Answers to each search, in order; past the end, nothing.
    pub searches: Mutex<VecDeque<XResult<Vec<XPost>>>>,
    pub trends: Option<Vec<Trend>>,
    pub searched: Mutex<Vec<(String, SearchOptions)>>,
    pub trend_asks: Mutex<Vec<TrendOptions>>,
}

impl FakeReads {
    pub fn answering(searches: impl IntoIterator<Item = XResult<Vec<XPost>>>) -> Self {
        Self {
            searches: Mutex::new(searches.into_iter().collect()),
            ..Self::default()
        }
    }

    pub fn searched(&self) -> Vec<(String, SearchOptions)> {
        self.searched.lock().unwrap().clone()
    }
}

#[async_trait]
impl XReads for FakeReads {
    async fn search_posts(&self, query: &str, options: &SearchOptions) -> XResult<Vec<XPost>> {
        self.searched
            .lock()
            .unwrap()
            .push((query.to_owned(), options.clone()));
        self.searches
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Ok(Vec::new()))
    }
    async fn get_post(&self, _id: &PostId) -> XResult<Option<XPost>> {
        Ok(None)
    }
    async fn get_conversation(&self, _id: &PostId, _max: Option<u32>) -> XResult<Vec<XPost>> {
        Ok(Vec::new())
    }
    async fn trends(&self, options: TrendOptions) -> XResult<Vec<Trend>> {
        self.trend_asks.lock().unwrap().push(options);
        match &self.trends {
            Some(trends) => Ok(trends.clone()),
            None => Err(teeny_x::XError::Missing("no trends on this tier".into())),
        }
    }
    async fn post_metrics(&self, _ids: &[PostId]) -> XResult<Vec<OwnPostMetrics>> {
        Ok(Vec::new())
    }
    async fn follows_account(&self, _user_id: &UserId) -> Option<bool> {
        None
    }
    async fn user_by_username(&self, _username: &str) -> XResult<Option<XUser>> {
        Ok(None)
    }
}

/// A poster that keeps every post, answers with ids `9001`, `9002`...,
/// and can be told to refuse.
#[derive(Default)]
pub struct Recorder {
    pub posts: Mutex<Vec<OwnPost>>,
    pub refuse: Option<String>,
}

impl Recorder {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn posts(&self) -> Vec<OwnPost> {
        self.posts.lock().unwrap().clone()
    }
}

#[async_trait]
impl OwnPoster for Recorder {
    async fn post(&self, post: OwnPost) -> Result<Posted, PostFailure> {
        if let Some(reason) = &self.refuse {
            return Err(PostFailure::Refused(reason.clone()));
        }
        let mut posts = self.posts.lock().unwrap();
        posts.push(post.clone());
        Ok(Posted {
            id: format!("{}", 9_000 + posts.len()),
            text: post.text,
            dry_run: false,
        })
    }
}

/// A post as the tests write them.
pub fn x_post(id: &str, text: &str) -> XPost {
    XPost::new(id, text)
}

/// Memory that answers every recall the same way and keeps what it is told.
pub struct Remembers {
    pub answer: Result<String, String>,
    pub recalled: Mutex<Vec<String>>,
    pub captured: Mutex<Vec<teeny_memory::Experience>>,
}

impl Remembers {
    pub fn saying(answer: &str) -> Arc<Self> {
        Arc::new(Self {
            answer: Ok(answer.to_owned()),
            recalled: Mutex::default(),
            captured: Mutex::default(),
        })
    }

    pub fn down() -> Arc<Self> {
        Arc::new(Self {
            answer: Err("memory is down".to_owned()),
            recalled: Mutex::default(),
            captured: Mutex::default(),
        })
    }

    pub fn queries(&self) -> Vec<String> {
        self.recalled.lock().unwrap().clone()
    }

    /// `(kind, payload)` of every activity captured, and the idempotency key.
    pub fn activities(&self) -> Vec<(String, String, serde_json::Value)> {
        self.captured
            .lock()
            .unwrap()
            .iter()
            .filter_map(|experience| match &experience.content {
                teeny_memory::Content::Json { data } => Some((
                    data["kind"].as_str().unwrap_or("").to_owned(),
                    experience.idempotency_key.clone(),
                    data["payload"].clone(),
                )),
                teeny_memory::Content::Message { .. } => None,
            })
            .collect()
    }
}

#[async_trait]
impl teeny_memory::Memory for Remembers {
    async fn capture(&self, experience: teeny_memory::Experience) -> teeny_memory::Result<()> {
        if self.answer.is_err() {
            return Err(teeny_memory::MemoryError::InvalidInput("memory is down".into()));
        }
        self.captured.lock().unwrap().push(experience);
        Ok(())
    }

    async fn recall(
        &self,
        query: &str,
        _person: Option<&teeny_memory::Person>,
    ) -> teeny_memory::Result<String> {
        self.recalled.lock().unwrap().push(query.to_owned());
        self.answer
            .clone()
            .map_err(teeny_memory::MemoryError::InvalidInput)
    }
}

pub fn at(text: &str) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339(text).unwrap().to_utc()
}

/// A context at `now` whose cadence reads `ledger` on the same clock.
pub fn ctx_at(
    now: chrono::DateTime<chrono::Utc>,
    ledger: Arc<teeny_store::posts::InMemoryPostLedger>,
) -> (crate::ctx::JobCtx, Arc<teeny_cadence::clock::ManualClock>) {
    ctx_with(now, ledger, |_| {})
}

/// [`ctx_at`], with the clients changed by `change` first.
pub fn ctx_with(
    now: chrono::DateTime<chrono::Utc>,
    ledger: Arc<teeny_store::posts::InMemoryPostLedger>,
    change: impl FnOnce(&mut teeny_agent::deps::Deps),
) -> (crate::ctx::JobCtx, Arc<teeny_cadence::clock::ManualClock>) {
    use teeny_cadence::cadence::{Cadence, CadenceRules};
    let clock = Arc::new(teeny_cadence::clock::ManualClock::new(now));
    let mut deps = teeny_agent::deps::Deps::fake();
    deps.cadence = Arc::new(Cadence::new(
        ledger,
        CadenceRules {
            min_gap: chrono::TimeDelta::hours(4),
            tolerance: chrono::TimeDelta::minutes(10),
        },
        clock.clone(),
    ));
    change(&mut deps);
    let base = crate::ctx::JobCtx::fake(Arc::new(crate::runner::ScriptedRunner::new()));
    let ctx = crate::ctx::JobCtx::new(
        Arc::new(deps),
        base.runner.clone(),
        base.config.clone(),
        base.journal.clone(),
    )
    .with_clock(clock.clone());
    (ctx, clock)
}

// --- the morning's fakes ------------------------------------------------------

use async_trait::async_trait as fake_async;
use teeny_media::reference::ReferenceSeed;
use teeny_media::types::{
    GeneratedImage, GeneratedVideo, ImageAspect, ImageGenerator, ImageInput, ImageReader,
    MimeType, VideoGenerator, VideoRequest,
};

use super::news::{NewsSource, Story, StoryKind};
use super::pipeline::Reference;

pub const PNG: [u8; 4] = [0x89, 0x50, 0x4e, 0x47];

pub fn story(title: &str, excerpt: &str) -> Story {
    Story {
        title: title.into(),
        url: format!("https://news.example/{}", title.len()),
        source: "news.example".into(),
        published_at: None,
        excerpt: excerpt.into(),
        topic: "ai".into(),
        kind: StoryKind::News,
        metrics: None,
        media: None,
    }
}

pub struct News;

#[fake_async]
impl NewsSource for News {
    async fn search(&self, _topic: &str, _days: u32, _limit: u32) -> Result<Vec<Story>, String> {
        Ok(vec![
            story("An open-source agent runtime crossed 40k stars", "Builders are flocking."),
            story("A wallet shipped account abstraction", ""),
        ])
    }
    async fn read(&self, _url: &str, _max: usize) -> String {
        "The runtime crossed 40,000 stars on Saturday.".into()
    }
}

#[derive(Default)]
pub struct Draw {
    pub prompts: Mutex<Vec<String>>,
}

#[fake_async]
impl ImageGenerator for Draw {
    async fn generate(&self, prompt: &str, aspect: ImageAspect) -> teeny_media::Result<GeneratedImage> {
        assert_eq!(aspect, ImageAspect::Square);
        self.prompts.lock().unwrap().push(prompt.to_owned());
        Ok(GeneratedImage {
            data: PNG.to_vec(),
            mime_type: MimeType::Png,
        })
    }
}

/// A camera that answers from a queue: `true` films, `false` fails.
pub struct Film {
    takes: Mutex<VecDeque<bool>>,
    pub asked: Mutex<Vec<VideoRequest>>,
}

impl Film {
    pub fn new(takes: impl IntoIterator<Item = bool>) -> Arc<Self> {
        Arc::new(Self {
            takes: Mutex::new(takes.into_iter().collect()),
            asked: Mutex::default(),
        })
    }
}

#[fake_async]
impl VideoGenerator for Film {
    async fn generate(&self, request: VideoRequest) -> teeny_media::Result<GeneratedVideo> {
        self.asked.lock().unwrap().push(request);
        if self.takes.lock().unwrap().pop_front().unwrap_or(true) {
            Ok(GeneratedVideo {
                data: vec![0, 0, 0, 0x18],
                seconds: Some(12),
            })
        } else {
            Err(teeny_media::Error::failed("no provider could accept this job right now"))
        }
    }
}

pub struct Eyes(pub Mutex<VecDeque<&'static str>>);

#[fake_async]
impl ImageReader for Eyes {
    async fn describe(&self, _image: ImageInput, _question: Option<&str>) -> teeny_media::Result<String> {
        Ok(self.0.lock().unwrap().pop_front().unwrap_or("{\"ok\": true}").to_owned())
    }
}

pub struct Sheet;

#[fake_async]
impl Reference for Sheet {
    async fn sheet(&self) -> Option<String> {
        Some("Tiny is a small yellow robot.".into())
    }
    async fn seed(&self, _random: f64) -> Option<ReferenceSeed> {
        Some(ReferenceSeed {
            url: "assets/tiny/tiny-love.jpg".into(),
            look: "Tiny hugging a heart, warm palette".into(),
        })
    }
}

pub const PICK: &str = r#"{"story": 1, "headline": "An agent runtime hit 40k stars", "topic": "ai", "angle": "open source wins", "visual": "a star counter ticking past 40k"}"#;
pub const SCENE: &str = r#"{"scene": "Tiny sits at a sunrise desk, a mug in hand, watching a star counter tick up on a tiny screen.", "storyCue": "the counter reading 40k stars", "alt": "Tiny at a desk at dawn."}"#;
pub const SCRIPT: &str = r#"{"script": "0-4s: static shot, Tiny sips from the mug as the sun rises. 4-10s: slow push in as the counter ticks. 10-12s: Tiny raises the mug to camera.", "summary": "Tiny sips coffee at sunrise, then raises the mug to say gm"}"#;
pub const CAPTION: &str = r#"{"text": "gm builders.\n\nan open-source agent runtime crossed 40k stars overnight, reportedly. small robots, big repos.\n\nin today's clip Tiny raises a mug to all of you."}"#;
pub const FRESH: &str = r#"{"said": "", "reason": "nothing like it in memory", "verdict": "fresh"}"#;

/// Answers each stage by the shape it was asked for; `overrides` replaces a
/// stage's answer by its name, one answer per call, the last repeating.
pub fn staged(overrides: Vec<(&'static str, Vec<&'static str>)>) -> Arc<Staged> {
    let overrides: Mutex<Vec<(&'static str, VecDeque<&'static str>)>> = Mutex::new(
        overrides
            .into_iter()
            .map(|(stage, answers)| (stage, answers.into_iter().collect()))
            .collect(),
    );
    Arc::new(Staged::new(move |prompt| {
        let stage = if prompt.contains("\"brief\": string") {
            "brief"
        } else if prompt.contains("\"verdict\": \"fresh\"|\"repeat\"|\"stale\"") {
            "fresh"
        } else if prompt.contains("\"story\": number") {
            "pick"
        } else if prompt.contains("\"scene\": string") {
            "scene"
        } else if prompt.contains("\"script\": string") {
            "script"
        } else if prompt.contains("\"text\": string") {
            "caption"
        } else {
            return Err(format!("unexpected stage: {}", &prompt[..60.min(prompt.len())]));
        };
        let mut overrides = overrides.lock().unwrap();
        if let Some((_, answers)) = overrides.iter_mut().find(|(name, _)| *name == stage) {
            let answer = if answers.len() > 1 {
                answers.pop_front().unwrap()
            } else {
                answers[0]
            };
            return Ok(answer.to_owned());
        }
        Ok(match stage {
            "brief" => r#"{"brief": "Tiny in a bucket hat and sunglasses, glowing nodes across the body, polished 3D, warm gold light."}"#,
            "fresh" => FRESH,
            "pick" => PICK,
            "scene" => SCENE,
            "script" => SCRIPT,
            _ => CAPTION,
        }
        .to_owned())
    }))
}
