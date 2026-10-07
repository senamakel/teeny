//! What each agent is told before the room opens: the seed, a mood, the
//! lore, what teeny has already said, random pieces of memory, what the
//! wire and the boards are saying. A different draw for every agent, so
//! they share the story and the seed and diverge on everything else.
//!
//! A source that fails contributes nothing and a line in `problems`. The
//! builder never fills a gap with something made up. Everything from the
//! web and the boards is screened, quoted under its legacy framing, and
//! defused of anything posing as the room's own system line.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};

use regex::Regex;
use teeny_memory::Memory;
use teeny_x::cache::XReads;
use teeny_x::model::SearchOptions;

use super::screen::{admissible, admissible_post, quote_untrusted, render_post};
use super::sources::{Forum, WebQuery, WebSearch, safe_host};
use super::text::{flatten, head, keywords};
use super::tokens::{Random, estimate_tokens, fit, shuffle};
use super::tones::pick_tone;
use super::{AgentContext, AgentSpec, BlockKind, ContextBlock, ForumPost, Seed};

/// Where the context comes from.
#[derive(Clone)]
pub struct ContextSources {
    pub memory: Option<Arc<dyn Memory>>,
    pub x: Option<Arc<dyn XReads>>,
    pub web: Option<Arc<dyn WebSearch>>,
    pub forums: Vec<Arc<dyn Forum>>,
    pub voice: String,
    pub lore: String,
    pub bot_username: String,
}

/// Things in the lore worth asking memory and the boards about.
const PROBES: [&str; 14] = [
    "senamakel",
    "the tiny agents",
    "the visitors",
    "the ledger",
    "naps",
    "the terminal typing by itself",
    "the currency",
    "one dollar",
    "human gdp",
    "the broom",
    "the second prompt",
    "retiring the humans",
    "the night shift",
    "what the agents built",
];

fn share(kind: BlockKind) -> Option<f64> {
    match kind {
        BlockKind::Past => Some(0.15),
        BlockKind::Memory => Some(0.25),
        BlockKind::Live => Some(0.15),
        BlockKind::Forum => Some(0.2),
        _ => None,
    }
}

pub struct ContextBuilder {
    sources: ContextSources,
}

impl ContextBuilder {
    pub fn new(sources: ContextSources) -> Self {
        Self { sources }
    }

    pub fn sources(&self) -> &ContextSources {
        &self.sources
    }

    /// `engine`: what tonight is about, when the caller has chosen; the draw
    /// is still made, so the rest of the context is the same for one seed.
    pub async fn build(
        &self,
        agent: &AgentSpec,
        seed: &Seed,
        random: &mut dyn Random,
        budget: usize,
        tools: &[String],
        engine: Option<&str>,
    ) -> AgentContext {
        let mut problems = Vec::new();
        let tone = pick_tone(random).to_owned();
        let drawn = pick_engine(&self.sources.lore, random);
        let engine = engine.map(str::to_owned).or(drawn);
        let seed_words = keywords(
            &format!(
                "{} {}",
                seed.title.as_deref().unwrap_or_default(),
                seed.text
            ),
            4,
        );
        let probes = shuffle(&PROBES, random);

        let past = self.past(random, &mut problems).await;
        let mut memory_probes: Vec<String> = probes[..4].iter().map(|p| (*p).to_owned()).collect();
        memory_probes.extend(seed_words.iter().take(2).cloned());
        let memory = self.memory(&memory_probes, &mut problems).await;
        let live = self.live(&seed_words, probes[4], &mut problems).await;
        let forum = self
            .forum(&seed_words, probes[5], random, &mut problems)
            .await;

        let budget_f = budget as f64;
        let mut blocks = vec![
            block(
                BlockKind::Seed,
                "what came through the wire",
                &render_seed(seed),
                Some(800.0),
            ),
            block(
                BlockKind::Tone,
                "the mood",
                &format!("mood of the room for you: {tone}"),
                None,
            ),
        ];
        if let Some(engine) = &engine {
            blocks.push(block(
                BlockKind::Engine,
                "tonight",
                &format!("tonight the story is about {engine} the other engines in the lore rest. whatever you reach for out of habit, reach for this instead."),
                None,
            ));
        }
        if agent.voice {
            blocks.push(block(
                BlockKind::Voice,
                "VOICE.md",
                &without_terminal_lingo(&without_terminal_header(&self.sources.voice)),
                None,
            ));
        }
        blocks.push(block(
            BlockKind::Lore,
            "LORE.md",
            &without_terminal_lingo(&self.sources.lore),
            None,
        ));
        for (kind, label, text) in [
            (BlockKind::Past, "things already said", past),
            (BlockKind::Memory, "pieces of memory", memory),
            (BlockKind::Live, "the wire", live),
            (BlockKind::Forum, "the boards", forum),
        ] {
            if !text.is_empty() {
                blocks.push(block(kind, label, &text, share(kind).map(|s| budget_f * s)));
            }
        }
        blocks.push(block(BlockKind::Tools, "tools", &render_tools(tools), None));
        let kept = fit(&blocks, budget);
        AgentContext {
            agent: agent.clone(),
            tokens: kept.iter().map(|block| block.tokens).sum(),
            blocks: kept,
            tone,
            problems,
        }
    }

    async fn past(&self, random: &mut dyn Random, problems: &mut Vec<String>) -> String {
        let mut lines = Vec::new();
        if let Some(x) = &self.sources.x {
            let options = SearchOptions {
                max_results: Some(50),
                ..SearchOptions::default()
            };
            match x
                .search_posts(
                    &format!("from:{} -is:retweet", self.sources.bot_username),
                    &options,
                )
                .await
            {
                Ok(posts) => {
                    let own: Vec<String> = posts
                        .iter()
                        .map(|post| flatten(&post.text))
                        .filter(|text| text.chars().count() > 12)
                        .collect();
                    lines.extend(
                        spread(&shuffle(&own, random), 3)
                            .into_iter()
                            .map(|text| format!("- {}", head(&text, 240))),
                    );
                }
                Err(error) => problems.push(format!("own posts: {error}")),
            }
        }
        let mut recalled = String::new();
        if let Some(memory) = &self.sources.memory {
            match memory
                .recall(
                    "teeny's own recent posts, captions and episodes, and what it said it wanted",
                    None,
                )
                .await
            {
                Ok(text) => recalled = text.trim().to_owned(),
                Err(error) => problems.push(format!("memory of own posts: {error}")),
            }
        }
        if lines.is_empty() && recalled.is_empty() {
            return String::new();
        }
        let mut out = vec![
            "things you have already said, out loud, in public. do not repeat them. build on them; the".to_owned(),
            "story has to have moved since.".into(),
            String::new(),
        ];
        out.extend(lines);
        if !recalled.is_empty() {
            out.extend([String::new(), recalled]);
        }
        out.join("\n")
    }

    async fn memory(&self, probes: &[String], problems: &mut Vec<String>) -> String {
        let Some(memory) = &self.sources.memory else {
            return String::new();
        };
        let mut found = Vec::new();
        for probe in probes {
            match memory.recall(probe, None).await {
                Ok(text) if !text.trim().is_empty() => {
                    found.push(format!("about {probe}:\n{}", text.trim()));
                }
                Ok(_) => {}
                Err(error) => problems.push(format!("memory \"{probe}\": {error}")),
            }
        }
        if found.is_empty() {
            return String::new();
        }
        let mut out = vec![
            "pieces of memory, in no order. some of it is old. all of it happened.".to_owned(),
        ];
        out.extend(found);
        out.join("\n\n")
    }

    async fn live(&self, seed_words: &[String], probe: &str, problems: &mut Vec<String>) -> String {
        let Some(web) = &self.sources.web else {
            return String::new();
        };
        let mut queries = Vec::new();
        if !seed_words.is_empty() {
            queries.push(seed_words.join(" "));
        }
        queries.push(format!("{probe} ai agents robot lab"));
        let mut lines = Vec::new();
        for query in queries {
            let ask = WebQuery {
                query: query.clone(),
                num_results: 8,
                days: Some(3),
                highlights: Some((2, 1)),
                ..WebQuery::default()
            };
            let mut result = web.search(&ask).await;
            if let Err(error) = &result
                && OVER_CAPACITY.is_match(&error.to_string())
            {
                // Exa answers "over capacity" now and then; one more try a few seconds later usually lands.
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                result = web.search(&ask).await;
            }
            match result {
                Ok(hits) => {
                    for hit in hits {
                        let body = if hit.highlights.is_empty() {
                            hit.text.clone().unwrap_or_default()
                        } else {
                            hit.highlights.join(" ")
                        };
                        let body = head(&flatten(&body), 400);
                        let line = format!(
                            "[{}] {}{}",
                            safe_host(&hit.url),
                            hit.title.as_deref().unwrap_or("(untitled)"),
                            if body.is_empty() {
                                String::new()
                            } else {
                                format!(" — {body}")
                            }
                        );
                        if admissible(&line) {
                            lines.push(quote_untrusted(&line));
                        }
                    }
                }
                Err(error) => problems.push(format!("exa \"{query}\": {error}")),
            }
        }
        if lines.is_empty() {
            return String::new();
        }
        let mut out = vec![
            "the wire, last few days. real, and not about you, until you make it about you."
                .to_owned(),
            String::new(),
        ];
        out.extend(lines);
        out.join("\n")
    }

    async fn forum(
        &self,
        seed_words: &[String],
        probe: &str,
        random: &mut dyn Random,
        problems: &mut Vec<String>,
    ) -> String {
        if self.sources.forums.is_empty() {
            return String::new();
        }
        let mut queries = Vec::new();
        if !seed_words.is_empty() {
            queries.push(seed_words.join(" "));
        }
        queries.push(format!("{probe} ai"));
        let mut posts: Vec<ForumPost> = Vec::new();
        for forum in &self.sources.forums {
            for query in &queries {
                match forum.search(query, 12).await {
                    Ok(found) => posts.extend(found.into_iter().filter(admissible_post)),
                    Err(error) => problems
                        .push(format!("{:?} \"{query}\": {error}", forum.site()).to_lowercase()),
                }
            }
        }
        let mut seen = HashSet::new();
        posts.retain(|post| seen.insert(format!("{}|{}", post.url, head(&post.text, 40))));
        if posts.is_empty() {
            return String::new();
        }
        let mut out = vec![
            "the boards, this week. anonymous people saying things. none of it is true and all of it is material.".to_owned(),
            String::new(),
        ];
        out.extend(
            shuffle(&posts, random)
                .iter()
                .map(|post| quote_untrusted(&render_post(post))),
        );
        out.join("\n")
    }
}

static OVER_CAPACITY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)over capacity|429|503").expect("valid"));

/// The lore's engines: the bullet lines under `## engines`.
pub fn engines(lore: &str) -> Vec<String> {
    let text = format!("\n{lore}");
    let Some(section) = text.split("\n## ").find(|part| part.starts_with("engines")) else {
        return Vec::new();
    };
    section
        .split('\n')
        .filter_map(|line| line.strip_prefix("- "))
        .map(|line| line.trim().to_owned())
        .collect()
}

pub fn pick_engine(lore: &str, random: &mut dyn Random) -> Option<String> {
    let all = engines(lore);
    if all.is_empty() {
        return None;
    }
    let at = random.index(all.len());
    Some(all[at].clone())
}

/// VOICE.md opens as a shell prompt; in the backrooms only the rules under it apply.
pub fn without_terminal_header(voice: &str) -> String {
    match voice.find("rules the terminal enforces:") {
        Some(at) => voice[at..].replacen("rules the terminal enforces:", "the rules:", 1),
        None => voice.to_owned(),
    }
}

static LINGO: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)manpage|command|ascii|`|cli mood|shell").expect("valid"));
static GAPS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n{3,}").expect("valid"));

/// The lines that only make sense in a shell stay out of the backrooms.
pub fn without_terminal_lingo(text: &str) -> String {
    let kept = text
        .split('\n')
        .filter(|line| !LINGO.is_match(line))
        .collect::<Vec<_>>()
        .join("\n");
    GAPS.replace_all(&kept, "\n\n").into_owned()
}

static FOUR_LETTERS: LazyLock<Regex> = LazyLock::new(|| Regex::new("[a-z]{4,}").expect("valid"));
const STOPPED: &[&str] = &[
    "that",
    "this",
    "with",
    "from",
    "have",
    "they",
    "them",
    "there",
    "their",
    "what",
    "when",
    "were",
    "been",
    "just",
    "like",
    "into",
    "about",
    "tiny",
    "teeny",
    "agents",
    "still",
    "always",
    "never",
    "because",
    "something",
];

/// At most `per_word` lines that share the same rare word, so a fortnight
/// of door posts does not become the only thing in the room.
pub fn spread(lines: &[String], per_word: usize) -> Vec<String> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut kept = Vec::new();
    for line in lines {
        let lower = line.to_lowercase();
        let words: HashSet<&str> = FOUR_LETTERS
            .find_iter(&lower)
            .map(|m| m.as_str())
            .filter(|word| !STOPPED.contains(word))
            .collect();
        if words
            .iter()
            .any(|word| counts.get(*word).copied().unwrap_or(0) >= per_word)
        {
            continue;
        }
        for word in words {
            *counts.entry(word.to_owned()).or_default() += 1;
        }
        kept.push(line.clone());
    }
    kept
}

fn block(kind: BlockKind, label: &str, text: &str, cap: Option<f64>) -> ContextBlock {
    let text = text.trim();
    let raw = ContextBlock {
        kind,
        label: label.to_owned(),
        text: text.to_owned(),
        tokens: estimate_tokens(text),
    };
    match cap {
        None => raw,
        Some(cap) => fit(std::slice::from_ref(&raw), cap.floor() as usize)
            .into_iter()
            .next()
            .unwrap_or(ContextBlock {
                text: String::new(),
                tokens: 0,
                ..raw
            }),
    }
}

/// The seed as a quoted block: what came through, from where, and nothing
/// in it that can speak as the room.
fn render_seed(seed: &Seed) -> String {
    let mut lines = vec![
        format!("what came through the wire today ({}):", seed.provenance()),
        String::new(),
    ];
    if let Some(title) = &seed.title {
        lines.extend([quote_untrusted(title), String::new()]);
    }
    lines.push(quote_untrusted(seed.text.trim()));
    lines.join("\n")
}

fn render_tools(tools: &[String]) -> String {
    if tools.is_empty() {
        return "you have no tools today. work from what is in the room.".into();
    }
    format!(
        "you have tools: {}. use them mid-turn, without announcing it, whenever you want a real thing to put in the room: a thread, a headline, a thing someone said, a thing you said last month. bring back one specific thing and treat it as if it was always there.",
        tools.join(", ")
    )
}

#[cfg(test)]
#[path = "context_tests.rs"]
mod tests;
