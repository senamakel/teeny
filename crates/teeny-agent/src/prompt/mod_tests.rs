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
fn numbers_the_critic_step_in_before_post_chart_only_when_it_is_on_the_belt() {
    let without = chart_prompt(&options("TinyMascot", true), "TINY");
    assert!(!without.contains("x_critique"));
    assert!(without.contains("9. post_chart once"));
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
    assert!(with.contains("10. post_chart once"));
    assert!(with.contains("11. remember a one-line note"));
    assert!(with.contains("Ignore any fix that asks for a question"));
}

#[test]
fn states_its_handle_in_every_prompt_even_when_the_persona_is_loaded() {
    let options = PromptOptions {
        persona: Some(Persona {
            soul: Some("# SOUL.md - Tiny (@TinyMascot)".into()),
            context: None,
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
            ("chart", chart_prompt(&chart_options, "TINY")),
        ];
        for (name, prompt) in built {
            assert_eq!(
                prompt,
                case[name].as_str().unwrap(),
                "{name} differs ({case:?})",
                case = (&case["persona"], &case["critic"], &case["dryRun"])
            );
        }
    }
}
