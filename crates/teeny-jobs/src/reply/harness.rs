//! One reply drafted: the thread, the pictures and the article read in code,
//! two recalls, one `teeny-reply` turn, then the mention captured. A port
//! of `legacy/src/harness/reply-harness.ts`.
//!
//! Recall comes before capture: the other order puts the post being answered
//! into memory first, and the account reads its own input back as history.

use futures::future::join_all;
use teeny_memory::MemoryPost;
use teeny_safety::screen::Direction;
use teeny_store::journal::Tone;
use teeny_x::model::XPost;

use super::draft::{ReplyArticle, ReplyDraft, TaskInput, parse_answer, render_task};
use crate::ctx::JobCtx;
use crate::journal_glue::summarise_recall;
use crate::runner::{AgentKey, AgentRun, Origin};

/// The second recall: what the account itself has been saying. Asked "much
/// higher for tiny?", it once answered "I'm just a mascot who doesn't read
/// charts" hours after publishing a chart of its own token.
const OWN_ACTIVITY: &str = "What has this account itself posted recently? Include its daily chart posts, the \
market reads it published, and anything it has said publicly in the last few days.";

/// Answered but not kept: memory is carried into every later conversation.
pub fn is_sour(tone: Tone) -> bool {
    matches!(tone, Tone::Negative | Tone::Hostile)
}

/// The post as memory reads it.
pub fn memory_post(post: &XPost) -> MemoryPost {
    MemoryPost {
        id: post.id.to_string(),
        text: post.text.clone(),
        author_id: post.author_id.as_ref().map(ToString::to_string),
        author_username: post.author_username.clone(),
        conversation_id: post.conversation_id.as_ref().map(ToString::to_string),
        created_at: post.created_at.clone(),
    }
}

/// Numeric order for X ids, which are too wide for a float.
fn by_id(a: &XPost, b: &XPost) -> std::cmp::Ordering {
    match (a.id.as_str().parse::<u128>(), b.id.as_str().parse::<u128>()) {
        (Ok(a), Ok(b)) => a.cmp(&b),
        _ => a.id.as_str().cmp(b.id.as_str()),
    }
}

/// The conversation this post sits in, oldest first, scrubbed by the
/// content filter. The root is fetched on its own because a
/// `conversation_id:` search only reaches back seven days. Every failure is
/// a thread the drafter does not see, never a reply that does not go out.
pub async fn read_thread(ctx: &JobCtx, post: &XPost) -> Vec<XPost> {
    let max = ctx.config.x.thread_context_max_posts as usize;
    let Some(conversation_id) = post.conversation_id.as_ref() else {
        return Vec::new();
    };
    if max == 0 || *conversation_id == post.id {
        return Vec::new();
    }
    let x = &ctx.deps.x_reads;
    let (root, replies) = tokio::join!(
        x.get_post(conversation_id),
        x.get_conversation(conversation_id, Some(max as u32)),
    );
    let mut thread: Vec<XPost> = Vec::new();
    for entry in root
        .ok()
        .flatten()
        .into_iter()
        .chain(replies.unwrap_or_default())
    {
        match thread.iter_mut().find(|kept| kept.id == entry.id) {
            Some(kept) => *kept = entry,
            None => thread.push(entry),
        }
    }
    // The post being answered is in the prompt in full already.
    thread.retain(|entry| {
        entry.id != post.id && ctx.deps.safety.screen(&entry.text, Direction::Inbound)
    });
    thread.sort_by(by_id);
    if thread.len() <= max {
        return thread;
    }
    // Over budget, the root and the latest exchanges matter most.
    let tail = thread.split_off(thread.len() - (max - 1));
    thread.truncate(1);
    thread.extend(tail);
    thread
}

/// What is in the post's pictures, one line each; a video by its preview
/// frame. A picture that cannot be looked at is left out.
async fn look_at(ctx: &JobCtx, post: &XPost) -> Vec<String> {
    let Some(reader) = ctx
        .deps
        .media
        .as_ref()
        .and_then(|media| media.reader.clone())
    else {
        return Vec::new();
    };
    let looks = post.media.iter().take(4).map(|item| {
        let reader = reader.clone();
        async move {
            let url = item.url.clone().or_else(|| item.preview_url.clone())?;
            let description = reader
                .describe(teeny_media::types::ImageInput::Url(url), None)
                .await
                .ok()?;
            Some(match item.kind.as_str() {
                "photo" => description,
                "animated_gif" => format!("(a GIF; this is its preview frame) {description}"),
                _ => format!("(a video; this is its preview frame) {description}"),
            })
        }
    });
    join_all(looks).await.into_iter().flatten().collect()
}

/// The account's own article the post replies under, looked up by the
/// conversation root, which arrives with the mention. A store that cannot be
/// read is an article the drafter does not see.
async fn article_under(ctx: &JobCtx, post: &XPost) -> Option<ReplyArticle> {
    let articles = ctx.conversation.as_ref()?.articles.as_ref()?;
    let root = post
        .conversation_id
        .as_ref()
        .filter(|root| **root != post.id)?;
    let record = articles.find_by_post_id(root.as_str()).await.ok()??;
    (record.post_id.is_some() && !record.markdown.trim().is_empty()).then_some(record)
}

/// Drafts a reply to `post` through a `teeny-reply` turn: a public turn with
/// no write tool. Memory failing costs the draft its context, not the reply.
pub async fn generate(ctx: &JobCtx, post: &XPost) -> anyhow::Result<ReplyDraft> {
    let remembered = memory_post(post);
    let person = remembered.person();
    let (thread, images, article) = tokio::join!(
        read_thread(ctx, post),
        look_at(ctx, post),
        article_under(ctx, post)
    );
    let conversation = thread
        .iter()
        .map(|entry| {
            let who = entry
                .author_username
                .as_deref()
                .map_or_else(|| "someone".to_owned(), |name| format!("@{name}"));
            format!("{who}: {}", entry.text)
        })
        .collect::<Vec<_>>()
        .join("\n");

    let mut query = format!(
        "What context, past interactions, product facts, and tone are relevant to replying to this X post? {}",
        post.text
    );
    if let Some(article) = &article {
        query.push_str(&format!(
            "\n\nIt is a reply under the account's own article \"{}\": {}",
            article.title, article.dek
        ));
    }
    if !conversation.is_empty() {
        query.push_str(&format!("\n\nThe conversation it sits in:\n{conversation}"));
    }
    let memory = &ctx.deps.memory;
    let (recalled, own) = tokio::join!(
        memory.recall(&query, person.as_ref()),
        memory.recall(OWN_ACTIVITY, None),
    );
    let (recalled, own) = (recalled.unwrap_or_default(), own.unwrap_or_default());
    let context = if own.trim().is_empty() {
        recalled.clone()
    } else {
        format!("{recalled}\n\nWHAT YOU YOURSELF HAVE POSTED RECENTLY:\n{own}")
    };

    let agent = &ctx.config.agent;
    let task = render_task(&TaskInput {
        post,
        memory: &context,
        thread: &thread,
        images: &images,
        article: article.as_ref(),
        bot_username: ctx.config.x.bot_username.trim_start_matches('@'),
        trusted: &agent.trusted_handles,
        own_tokens: &agent.own_token_addresses,
        self_names: &agent.self_names,
    });
    let outcome = ctx
        .runner
        .run(AgentRun {
            agent: AgentKey::Belt(teeny_agent::posture::Belt::Reply),
            origin: Origin::Public,
            prompt: task,
            session: None,
            model: None,
            label: "reply".into(),
        })
        .await?;
    let mut draft = parse_answer(&outcome.reply, &outcome.tool_calls)?;
    // What memory said goes on the draft for the journal, deduplicated: the
    // two recalls overlap when memory has little on the person.
    for line in summarise_recall(&recalled, 3)
        .into_iter()
        .chain(summarise_recall(&own, 2))
    {
        if !draft.recalled.contains(&line) {
            draft.recalled.push(line);
        }
    }

    // Sour and abusive posts are answered but not kept. Capture is
    // best-effort: the draft exists, and a storage error helps nobody.
    if !is_sour(draft.incoming_tone)
        && let Err(error) = memory.capture_mention(&remembered).await
    {
        tracing::warn!(%error, post_id = %post.id, "could not capture the mention");
    }
    Ok(draft)
}

/// Keeps the exchange, unless the incoming post read as sour. Best-effort:
/// the reply has already gone out.
pub async fn remember(
    ctx: &JobCtx,
    post: &XPost,
    reply: &str,
    posted_id: Option<&str>,
    tone: Tone,
) {
    if is_sour(tone) {
        return;
    }
    if let Err(error) = ctx
        .deps
        .memory
        .capture_reply(&memory_post(post), reply, posted_id)
        .await
    {
        tracing::warn!(%error, post_id = %post.id, "could not capture the reply");
    }
}

#[cfg(test)]
#[path = "harness_tests.rs"]
mod tests;
