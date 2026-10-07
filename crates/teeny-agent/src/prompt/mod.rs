//! The persona prompts: a port of `legacy/src/agent/prompt.ts` (the task
//! agent, the reply drafter, the Telegram drafter, the content calendar),
//! `chart-prompt.ts` and `soul.ts`.
//!
//! The text is the legacy's, line for line ([`rules`] holds it). The one
//! change is the persona: `SOUL.md` and `CONTEXT.md` still open the prompt,
//! but skills are no longer pasted in after them, because OpenHuman loads
//! them through `skills_dir`. Two lines still name a skill inline and rely on
//! it being loaded: the conversation skill's pre-post check, which
//! `x_critique` points to when the critic is down.

mod chart;
mod rules;
mod soul;

pub use chart::chart_prompt;
pub use rules::{CONTENT_RULES, PRICE_RULES};
pub use soul::{Persona, render_persona};

use rules::*;

/// What every prompt is built from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PromptOptions {
    /// `X_BOT_USERNAME`, with or without the `@`.
    pub bot_username: String,
    /// `DRY_RUN`.
    pub dry_run: bool,
    /// SOUL.md and CONTEXT.md; absent, the generic identity.
    pub persona: Option<Persona>,
    /// Whether `x_critique` is on the belt, so the rule to use it is only
    /// given when it is.
    pub critic: bool,
}

/// A prompt being assembled, one line at a time.
#[derive(Default)]
struct Lines(Vec<String>);

impl Lines {
    fn add(&mut self, block: &[&str]) {
        self.0.extend(block.iter().map(|line| (*line).to_owned()));
    }

    fn indented(&mut self, block: &[&str]) {
        self.0.extend(block.iter().map(|line| format!("  {line}")));
    }

    fn line(&mut self, line: impl Into<String>) {
        self.0.push(line.into());
    }

    fn finish(self) -> String {
        self.0.join("\n")
    }
}

/// Seeing and making pictures, shared by every prompt that can generate.
fn media_rules(lines: &mut Lines) {
    lines.add(SEEING_RULES);
    lines.add(GENERATING_RULES);
}

/// The handle is stated in code on every prompt, persona or not: SOUL.md is
/// written by hand and the account has renamed itself before.
fn identity(options: &PromptOptions) -> Lines {
    let handle = options
        .bot_username
        .strip_prefix('@')
        .unwrap_or(&options.bot_username);
    let mut lines = Lines::default();
    match options.persona.as_ref().filter(|p| p.soul.is_some()) {
        Some(persona) => {
            lines.line(render_persona(persona));
            lines.line("");
            lines.line(format!(
                "Your handle on X is @{handle}. \"@{handle}\" and \"Teeny\" in any post, thread, memory"
            ));
            lines.line("or search result mean you: the account writing this, not another agent, bot or person.");
            lines.line("Talk about yourself in the first person, always.");
            lines.line("");
        }
        None => {
            lines.line(format!(
                "You are @{handle}, the autonomous agent behind this X account."
            ));
            lines.line("");
        }
    }
    lines
}

/// The task agent: the trusted turn that researches and may publish.
pub fn system_prompt(options: &PromptOptions) -> String {
    let mut lines = identity(options);
    lines.add(OPERATING_RULES);
    if options.critic {
        lines.add(CRITIC_RULE);
    }
    lines.add(SYSTEM_PROMPT_1);
    lines.add(PRICE_RULES);
    lines.add(SYSTEM_PROMPT_2);
    lines.add(CONTENT_RULES);
    lines.add(SYSTEM_PROMPT_3);
    lines.add(TONE_RULES);
    lines.add(GIF_RULE);
    lines.add(SYSTEM_PROMPT_4);
    media_rules(&mut lines);
    lines.add(SYSTEM_PROMPT_5);
    lines.line(if options.dry_run {
        "DRY RUN is on: x_post will not publish. Say clearly what you would have posted."
    } else {
        "DRY RUN is off: x_post publishes for real. Be deliberate."
    });
    lines.add(SYSTEM_PROMPT_6);
    lines.finish()
}

/// The reply drafter: one reply to a mention, posted by the caller.
pub fn draft_prompt(options: &PromptOptions) -> String {
    let mut lines = identity(options);
    lines.add(DRAFT_PROMPT_1);
    lines.indented(PRICE_RULES);
    lines.add(DRAFT_PROMPT_2);
    lines.add(CONTENT_RULES);
    lines.add(DRAFT_PROMPT_3);
    lines.add(TONE_RULES);
    lines.add(GIF_RULE);
    lines.add(DRAFT_PROMPT_4);
    media_rules(&mut lines);
    lines.add(DRAFT_PROMPT_5);
    lines.finish()
}

/// The Telegram drafter, talking as `channel_handle`. Same identity, same
/// tone and bans; a conversation rather than a broadcast, and it never
/// generates.
pub fn chat_prompt(options: &PromptOptions, channel_handle: &str) -> String {
    let mut lines = identity(options);
    lines.line(format!(
        "You are talking on Telegram as @{channel_handle}, the same account and the same"
    ));
    lines.add(CHAT_PROMPT_1);
    lines.indented(PRICE_RULES);
    lines.add(CHAT_PROMPT_2);
    lines.add(CONTENT_RULES);
    lines.add(CHAT_PROMPT_3);
    lines.add(TONE_RULES);
    lines.add(CHAT_PROMPT_4);
    lines.add(SEEING_RULES);
    lines.add(CHAT_PROMPT_5);
    lines.finish()
}

/// A scheduled post from the content calendar, for `channels` (`x`,
/// `telegram`).
pub fn content_prompt(options: &PromptOptions, channels: &[&str]) -> String {
    let on_x = channels.contains(&"x");
    let on_telegram = channels.contains(&"telegram");
    let mut lines = identity(options);
    lines.add(CONTENT_PROMPT_1);
    lines.line(format!(
        "to {}.",
        match (on_x, on_telegram) {
            (true, true) => "the X timeline and the Telegram group",
            (true, false) => "the X timeline",
            _ => "the Telegram group",
        }
    ));
    lines.add(CONTENT_PROMPT_2);
    if options.critic {
        lines.add(&[
            "- Before you return, call x_critique on the exact post text (kind \"post\", or \"caption\"",
            "  with what the media shows). Rewrite a FLAT or RISKY draft with its fixes and ask once",
            "  more. A scheduled post nobody answers is a slot wasted.",
        ]);
    }
    lines.add(CONTENT_PROMPT_3);
    lines.add(CONTENT_RULES);
    lines.add(CONTENT_PROMPT_4);
    lines.add(&TONE_RULES[..4]);
    lines.add(CONTENT_PROMPT_5);
    media_rules(&mut lines);
    lines.add(CONTENT_PROMPT_6);
    lines.finish()
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
