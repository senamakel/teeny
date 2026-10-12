//! Port of legacy/test/persona.test.ts and the prompt cases of
//! self-knowledge.test.ts. One change from the legacy: skills are no longer
//! pasted into the prompt (OpenHuman loads them through `skills_dir`), so
//! the case that asserted every skill in both prompts now asserts none.

use std::path::PathBuf;

use super::*;

/// A persona directory, as the legacy fixture laid it out.
fn fixture(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("teeny-persona-{name}-{}", std::process::id()));
    std::fs::create_dir_all(root.join("skills/humanizer")).unwrap();
    std::fs::write(root.join("SOUL.md"), "# SOUL.md - Tiny\n\nYou are Tiny.").unwrap();
    std::fs::write(
        root.join("CONTEXT.md"),
        "# CONTEXT.md\n\n$TINY is the only token.",
    )
    .unwrap();
    std::fs::write(
        root.join("skills/humanizer/SKILL.md"),
        "# Humanizer\n\nNo em dashes.",
    )
    .unwrap();
    root
}

fn persona(name: &str) -> Persona {
    let root = fixture(name);
    Persona::load(root.join("SOUL.md"), root.join("CONTEXT.md")).unwrap()
}

fn options(bot: &str, dry_run: bool) -> PromptOptions {
    PromptOptions {
        bot_username: bot.into(),
        dry_run,
        ..PromptOptions::default()
    }
}

#[test]
fn reads_soul_and_context() {
    let persona = persona("reads");
    assert!(persona.soul.as_deref().unwrap().contains("You are Tiny."));
    assert!(
        persona
            .context
            .as_deref()
            .unwrap()
            .contains("$TINY is the only token.")
    );
}

#[test]
fn is_a_no_op_when_the_files_do_not_exist() {
    let missing = std::env::temp_dir().join("teeny-missing-persona");
    let persona = Persona::load(missing.join("SOUL.md"), missing.join("CONTEXT.md")).unwrap();
    assert_eq!(persona, Persona::default());
}

#[test]
fn puts_the_soul_and_context_into_both_prompts_and_no_skills() {
    let options = PromptOptions {
        persona: Some(persona("both")),
        ..options("TinyHumanAI", true)
    };
    for prompt in [system_prompt(&options), draft_prompt(&options)] {
        assert!(prompt.contains("You are Tiny."));
        assert!(prompt.contains("$TINY is the only token."));
        assert!(!prompt.contains("<skill"));
        assert!(!prompt.contains("No em dashes."));
    }
}

#[test]
fn falls_back_to_the_generic_identity_without_a_soul() {
    let prompt = system_prompt(&options("stonksrocks", false));
    assert!(!prompt.contains("x_critique"));
    let with_critic = system_prompt(&PromptOptions {
        critic: true,
        ..options("stonksrocks", false)
    });
    assert!(with_critic.contains("call\n  x_critique"));
    assert!(prompt.contains("@stonksrocks"));
    assert!(prompt.contains("DRY RUN is off"));
    assert!(prompt.starts_with(
        "You are @stonksrocks, the autonomous agent behind this X account.\n\nHow you work:"
    ));
}

#[test]
fn renders_nothing_for_an_empty_persona() {
    assert_eq!(render_persona(&Persona::default()), "");
}

#[test]
fn states_the_one_commercial_rule_in_both_prompts_even_without_a_context_file() {
    let options = options("TinyMascot", true);
    for prompt in [system_prompt(&options), draft_prompt(&options)] {
        assert!(prompt.contains("$TINY is the only token"));
        assert!(prompt.contains("ever endorse"));
        assert!(
            prompt.contains("ever use the word shill")
                || prompt.contains("not moving your own bag")
        );
    }
}

#[test]
fn the_drafter_carries_the_same_tone_rules_as_the_task_agent() {
    let options = options("TinyMascot", false);
    for prompt in [system_prompt(&options), draft_prompt(&options)] {
        assert!(prompt.contains("roast back"));
        assert!(prompt.contains("do not roast and"));
        assert!(prompt.contains("Reach for a GIF often"));
        assert!(prompt.contains("Build people up"));
        assert!(prompt.contains("Do not deflate people"));
    }
}

#[test]
fn numbers_the_critic_step_in_before_the_answer_only_when_it_is_on_the_belt() {
    let without = chart_prompt(&options("TinyMascot", true), "TINY");
    assert!(!without.contains("x_critique"));
    assert!(without.contains("9. Answer with the post and that chartId"));
    assert!(without.contains("It ends on your read, not on a question."));
    let with = chart_prompt(
        &PromptOptions {
            critic: true,
            ..options("TinyMascot", true)
        },
        "TINY",
    );
    assert!(with.contains("9. x_critique on the exact post text"));
    assert!(with.contains("$TINY on the timeframe"));
    assert!(with.contains("10. Answer with the post and that chartId"));
    assert!(with.contains("Ignore any fix that asks for a question"));
}

#[test]
fn the_chart_agent_is_never_told_to_post_or_remember_itself() {
    for critic in [false, true] {
        for dry_run in [false, true] {
            let prompt = chart_prompt(
                &PromptOptions {
                    critic,
                    dry_run,
                    ..options("TinyMascot", true)
                },
                "TINY",
            );
            assert!(!prompt.contains("post_chart"), "{prompt}");
            assert!(!prompt.contains("remember a"), "{prompt}");
            assert!(prompt.contains("You do not publish it"), "{prompt}");
        }
    }
}

#[test]
fn states_its_handle_in_every_prompt_even_when_the_persona_is_loaded() {
    let options = PromptOptions {
        persona: Some(Persona {
            soul: Some("# SOUL.md - Tiny (@TinyMascot)".into()),
            ..Persona::default()
        }),
        ..options("teenydroid", true)
    };
    for prompt in [system_prompt(&options), draft_prompt(&options)] {
        assert!(prompt.contains("Your handle on X is @teenydroid"));
        assert!(prompt.contains("first person"));
    }
}

#[test]
fn every_prompt_about_the_token_carries_the_goal_the_prophecy_and_no_disclaimer() {
    let options = options("teenydroid", true);
    let all = [
        system_prompt(&options),
        draft_prompt(&options),
        chat_prompt(&options, "teenythebot"),
        chart_prompt(&options, "TINY"),
    ];
    for prompt in &all {
        assert!(prompt.contains("$2B"));
        assert!(prompt.contains("GOAT"));
        let lower = prompt.to_lowercase();
        assert!(
            lower.contains("no \"nfa\"")
                || lower.contains("never write \"nfa\"")
                || lower.contains("no disclaimer")
        );
        for banned in [
            "carries \"nfa\"",
            "with \"nfa\" on",
            "ends with \"nfa\"",
            "no price predictions",
        ] {
            assert!(!lower.contains(banned), "{banned}");
        }
    }
    for prompt in &all[..3] {
        assert!(prompt.contains("$1 prophecy"));
    }
    assert!(content_prompt(&options, &["x"]).contains("never with \"nfa\""));
}

#[test]
fn the_chat_and_content_prompts_follow_their_room() {
    let options = options("teenydroid", false);
    let chat = chat_prompt(&options, "teenythebot");
    assert!(
        chat.contains("You are talking on Telegram as @teenythebot, the same account and the same")
    );
    assert!(chat.ends_with("\"MEDIA: none\"."));
    assert!(
        content_prompt(&options, &["x", "telegram"])
            .contains("to the X timeline and the Telegram group.")
    );
    assert!(content_prompt(&options, &["telegram"]).contains("to the Telegram group."));
    assert!(!content_prompt(&options, &["x"]).contains("x_critique"));
    let critic = PromptOptions {
        critic: true,
        ..options
    };
    assert!(content_prompt(&critic, &["x"]).contains("call x_critique on the exact post text"));
}

#[test]
fn reads_the_persona_paths_from_config() {
    let root = fixture("config");
    let mut agent = teeny_core::Config::from_vars(crate::deps::fake_vars())
        .unwrap()
        .agent;
    agent.soul_path = root.join("SOUL.md").display().to_string();
    agent.context_path = root.join("CONTEXT.md").display().to_string();
    assert!(
        Persona::from_config(&agent)
            .unwrap()
            .soul
            .unwrap()
            .contains("You are Tiny.")
    );
}

/// Every prompt, byte for byte against what the legacy builders returned
/// (`fixtures/prompts/generate.mts`).
#[test]
fn matches_the_legacy_prompts_byte_for_byte() {
    let golden: serde_json::Value =
        serde_json::from_str(include_str!("../../fixtures/prompts/prompts.json")).unwrap();
    for case in golden.as_array().unwrap() {
        let options = PromptOptions {
            bot_username: "@teenydroid".into(),
            dry_run: case["dryRun"].as_bool().unwrap(),
            critic: case["critic"].as_bool().unwrap(),
            persona: case["persona"].as_bool().unwrap().then(|| Persona {
                soul: Some("# SOUL.md - Tiny\n\nYou are Tiny.".into()),
                context: Some("# CONTEXT.md\n\n$TINY is the only token.".into()),
                ..Persona::default()
            }),
        };
        let chart_options = PromptOptions {
            bot_username: "teenydroid".into(),
            ..options.clone()
        };
        let built = [
            ("system", system_prompt(&options)),
            ("draft", draft_prompt(&options)),
            ("chat", chat_prompt(&options, "teenythebot")),
            ("contentX", content_prompt(&options, &["x"])),
            ("contentBoth", content_prompt(&options, &["x", "telegram"])),
            ("contentTelegram", content_prompt(&options, &["telegram"])),
        ];
        for (name, prompt) in built {
            assert_eq!(
                prompt,
                case[name].as_str().unwrap(),
                "{name} differs ({case:?})",
                case = (&case["persona"], &case["critic"], &case["dryRun"])
            );
        }
        let mut chart = case["chart"].as_str().unwrap().to_owned();
        for (legacy, now) in CHART_CHANGES {
            chart = chart.replace(legacy, now);
        }
        assert!(
            !chart.contains("post_chart"),
            "a legacy line was left: {chart}"
        );
        assert_eq!(chart_prompt(&chart_options, "TINY"), chart, "chart differs");
    }
}

/// The legacy chart prompt, less its own posting and remembering: the agent
/// answers with the post and the host publishes and remembers it.
const CHART_CHANGES: [(&str, &str); 6] = [
    (
        "on the timeframe\", before post_chart.",
        "on the timeframe\", before you answer.",
    ),
    (
        "DRY RUN is on: post_chart will not publish.",
        "DRY RUN is on: the post you answer with will not be published.",
    ),
    (
        "DRY RUN is off: post_chart publishes for real.",
        "DRY RUN is off: the post you answer with is published for real.",
    ),
    (
        "10. post_chart once, with that chartId.\n11. remember a one-line note for anything worth carrying forward that is not the post\n   itself: something a holder asked, a name worth knowing, a mood worth remembering.\n   The post text is stored for you automatically, so do not just restate it.\n",
        ANSWER_10,
    ),
    (
        "9. post_chart once, with that chartId.\n10. remember a one-line note for anything worth carrying forward that is not the post\n   itself: something a holder asked, a name worth knowing, a mood worth remembering.\n   The post text is stored for you automatically, so do not just restate it.\n",
        ANSWER_9,
    ),
    (
        "post_chart cuts\n  a closing question.",
        "the service cuts\n  a closing question.",
    ),
];

const ANSWER_10: &str = "10. Answer with the post and that chartId, in the shape the task gives. You do not publish it:\n   the service checks it and publishes it, and remembers what went out.\n";
const ANSWER_9: &str = "9. Answer with the post and that chartId, in the shape the task gives. You do not publish it:\n   the service checks it and publishes it, and remembers what went out.\n";

/// Port of "the learnings in the persona" in `legacy/test/learnings.test.ts`.
#[test]
fn the_learnings_reach_the_rendered_persona_and_a_rewrite_reaches_it_without_a_reload() {
    let root = fixture("learnings");
    let path = root.join("memory/LEARNINGS.md");
    let _ = std::fs::remove_file(&path);
    let persona = Persona::load(root.join("SOUL.md"), root.join("CONTEXT.md"))
        .unwrap()
        .with_learnings(&path);
    // A clone (the prompts hold one each) reads the same file.
    let held = persona.clone();

    assert_eq!(persona.learnings(), None);
    assert!(!render_persona(&held).contains("<learnings>"));

    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "# Learnings\n\n- @ana likes the lab lore\n").unwrap();
    let rendered = render_persona(&held);
    assert!(
        rendered.contains("<learnings>\n# Learnings\n\n- @ana likes the lab lore\n</learnings>"),
        "{rendered}"
    );
    // After the facts, before nothing else: SOUL.md and CONTEXT.md outrank it.
    assert!(
        rendered.find("$TINY is the only token").unwrap() < rendered.find("<learnings>").unwrap()
    );
    assert!(rendered.contains("SOUL.md and the facts above still outrank it"));

    // The same length, so only the modification time tells them apart.
    std::fs::write(&path, "# Learnings\n\n- @ana loves the lab lore\n").unwrap();
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(later)
        .unwrap();
    assert!(render_persona(&held).contains("@ana loves the lab lore"));

    // A brief that goes away is no brief, not an error.
    std::fs::remove_file(&path).unwrap();
    assert_eq!(held.learnings(), None);
}

#[test]
fn reads_the_brief_only_when_the_learnings_are_on() {
    let root = fixture("learnings-config");
    let config = teeny_core::Config::from_vars(
        crate::deps::fake_vars()
            .into_iter()
            .chain([("LEARNINGS_PATH", "/nonexistent/LEARNINGS.md")]),
    )
    .unwrap();
    let mut agent = config.agent.clone();
    agent.soul_path = root.join("SOUL.md").display().to_string();
    let on = Persona::from_settings(&agent, &config.learnings).unwrap();
    assert_eq!(
        on.learnings_path(),
        Some(std::path::Path::new("/nonexistent/LEARNINGS.md"))
    );
    let mut off = config.learnings.clone();
    off.enabled = false;
    assert_eq!(
        Persona::from_settings(&agent, &off)
            .unwrap()
            .learnings_path(),
        None
    );
}
