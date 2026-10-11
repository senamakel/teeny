//! The spec table as data. What a registered agent can actually reach is
//! proven against a real runtime in `tests/posture.rs`.

use std::path::Path;

use openhuman_embed::AgentTurnOrigin;
use openhuman_embed::TrustedAutomationSource;

use super::*;
use crate::deps::Deps;
use crate::prompt;

#[test]
fn every_belt_has_its_fixed_agent_id() {
    let ids: Vec<&str> = Belt::ALL.iter().map(|belt| agent_id(*belt)).collect();
    assert_eq!(
        ids,
        [
            "teeny-reply",
            "teeny-chat",
            "teeny-chart",
            "teeny-room",
            "teeny-operator",
            "teeny-drafter",
        ]
    );
}

#[test]
fn a_room_is_its_own_agent_per_persona() {
    assert_eq!(room_id("skeptic").unwrap(), "teeny-room-skeptic");
    assert_eq!(
        room_id("serial-producer").unwrap(),
        "teeny-room-serial-producer"
    );
    let longest = "a".repeat(48);
    assert_eq!(room_id(&longest).unwrap().len(), 59);
    for bad in ["", "Skeptic", "a b", "../x", &"a".repeat(49)] {
        assert!(room_id(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn only_the_research_belts_add_the_browser_and_nothing_else_is_built_in() {
    for belt in Belt::ALL {
        let expected: &[&str] = match belt {
            Belt::Operator | Belt::Room => &["browser"],
            _ => &[],
        };
        assert_eq!(built_ins(belt), expected, "{belt:?}");
    }
}

#[test]
fn the_scope_is_the_belt_as_built_plus_its_built_ins() {
    let deps = Deps::fake();
    for belt in Belt::ALL {
        let tools = belt.build(&deps);
        let mut expected: Vec<String> = tools.iter().map(|t| t.name().to_owned()).collect();
        expected.extend(built_ins(belt).iter().map(|name| (*name).to_owned()));
        assert_eq!(tool_scope(belt, &tools), expected, "{belt:?}");
    }
}

#[test]
fn public_belts_run_as_an_external_channel_and_the_rest_as_trusted() {
    for belt in Belt::ALL {
        let access = access(belt);
        match belt {
            Belt::Reply | Belt::Chat => assert!(
                matches!(
                    access.turn_origin(),
                    Some(AgentTurnOrigin::ExternalChannel { .. })
                ),
                "{belt:?}"
            ),
            _ => assert!(
                matches!(
                    access.turn_origin(),
                    Some(AgentTurnOrigin::TrustedAutomation {
                        source: TrustedAutomationSource::Workflow { .. },
                        ..
                    })
                ),
                "{belt:?}"
            ),
        }
    }
}

#[test]
fn each_belt_speaks_with_its_legacy_prompt() {
    let deps = Deps::fake();
    let inputs = PromptInputs::from_deps(&deps);
    let options = inputs.options();
    assert_eq!(options.bot_username, "teenydroid");
    assert!(options.critic, "the fake deps carry a critic");
    assert_eq!(
        belt_prompt(Belt::Reply, &inputs).as_deref(),
        Some(prompt::draft_prompt(&options).as_str())
    );
    assert_eq!(
        belt_prompt(Belt::Chat, &inputs).as_deref(),
        Some(prompt::chat_prompt(&options, "teenydroid").as_str())
    );
    assert_eq!(
        belt_prompt(Belt::Operator, &inputs).as_deref(),
        Some(prompt::system_prompt(&options).as_str())
    );
    assert_eq!(
        belt_prompt(Belt::Chart, &inputs).as_deref(),
        Some(prompt::chart_prompt(&options, &deps.settings.token_symbol).as_str())
    );
    assert_eq!(
        belt_prompt(Belt::Drafter, &inputs).as_deref(),
        Some(prompt::content_prompt(&options, &["x"]).as_str())
    );
    assert_eq!(
        belt_prompt(Belt::Room, &inputs),
        None,
        "a room speaks with its persona"
    );
}

#[test]
fn the_context_reads_the_ladder_the_model_the_skills_and_the_step_cap() {
    let mut vars = crate::deps::fake_vars();
    vars.extend([
        ("LLM_BASE_URL", "https://ladder.example/v1"),
        ("SKILLS_DIR", "my-skills"),
        ("AGENT_MAX_STEPS", "12"),
    ]);
    let config = teeny_core::Config::from_vars(vars).unwrap();
    let context = SpecContext::from_config(&config);
    assert_eq!(context.base_url, "https://ladder.example/v1");
    assert_eq!(context.api_key.expose(), "fake");
    assert_eq!(context.model, "chat");
    assert!(context.skills_dir.is_absolute());
    assert!(context.skills_dir.ends_with(Path::new("my-skills")));
    assert_eq!(context.max_iterations, 12);
}

#[test]
fn a_spec_carries_its_belts_id() {
    let deps = Deps::fake();
    let context =
        SpecContext::from_config(&teeny_core::Config::from_vars(crate::deps::fake_vars()).unwrap());
    let tools = Arc::new(Belt::Reply.build(&deps));
    let spec = belt_spec(Belt::Reply, tools, "prompt".into(), &context);
    assert_eq!(spec.id(), "teeny-reply");
}

#[test]
fn every_agent_keeps_its_own_turns_under_the_one_shared_memory_root() {
    let deps = Deps::fake();
    let mut vars = crate::deps::fake_vars();
    vars.push(("TEENY_MEMORY_ROOT", "team:teeny-shadow"));
    let context = SpecContext::from_config(&teeny_core::Config::from_vars(vars).unwrap());
    assert_eq!(context.memory_root, "team:teeny-shadow");
    for belt in [Belt::Reply, Belt::Chart, Belt::Operator] {
        let tools = Arc::new(belt.build(&deps));
        let spec = belt_spec(belt, tools, "prompt".into(), &context);
        let expected = MemoryBinding::new(agent_id(belt)).root("team:teeny-shadow");
        assert!(
            format!("{spec:?}").contains(&format!("{:?}", Some(&expected))),
            "{spec:?}"
        );
    }
    let room = memory_binding("teeny-room-gm", &context);
    assert_eq!(room.agent_id(), "teeny-room-gm");
    assert_eq!(room.root_namespace(), Some("team:teeny-shadow"));
}

#[test]
fn a_public_belt_is_handed_its_reads_only_and_parks_nothing() {
    let deps = Deps::fake();
    for belt in Belt::ALL {
        let built = belt.build(&deps);
        let handed = handed(belt, &built);
        if is_public(belt) {
            assert!(
                handed
                    .iter()
                    .all(|tool| tool.permission_level() < PermissionLevel::Write),
                "{belt:?}"
            );
            assert!(!access(belt).approval_gate_enabled(), "{belt:?}");
        } else {
            assert_eq!(handed.len(), built.len(), "{belt:?}");
        }
    }
}

#[test]
fn the_deny_rules_refuse_every_denied_family_and_admit_the_belts() {
    use tinytools::rules::{RuleContext, Surface, ToolSubject};
    let rules = deny_rules();
    let refused = |name: &str| {
        !rules
            .evaluate(
                &ToolSubject::named(name),
                &RuleContext::new(),
                Surface::Call,
                None,
            )
            .callable
    };
    for name in [
        "shell",
        "cron_add",
        "spawn_subagent",
        "delegate_x",
        "mcp_call_tool",
        "run_flow",
    ] {
        assert!(refused(name), "{name}");
    }
    let deps = Deps::fake();
    for belt in Belt::ALL {
        for name in tool_scope(belt, &belt.build(&deps)) {
            assert!(!refused(&name), "{belt:?}: {name}");
        }
    }
}
