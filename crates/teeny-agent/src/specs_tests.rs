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
            "teeny-learner",
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
fn only_the_rooms_add_the_browser_and_only_with_the_experiment_on() {
    for belt in Belt::ALL {
        let expected: &[&str] = match belt {
            Belt::Room => &["browser"],
            _ => &[],
        };
        assert_eq!(built_ins(belt, true), expected, "{belt:?}");
        assert!(built_ins(belt, false).is_empty(), "{belt:?}: off is off");
    }
}

#[test]
fn the_public_belts_and_the_learner_never_browse() {
    for belt in [Belt::Reply, Belt::Chat, Belt::Learner, Belt::Operator] {
        assert!(built_ins(belt, true).is_empty(), "{belt:?}");
    }
}

#[test]
fn the_scope_is_the_belt_as_built_plus_its_built_ins() {
    let deps = Deps::fake();
    for browse in [false, true] {
        for belt in Belt::ALL {
            let tools = belt.build(&deps);
            let mut expected: Vec<String> = tools.iter().map(|t| t.name().to_owned()).collect();
            expected.extend(
                built_ins(belt, browse)
                    .iter()
                    .map(|name| (*name).to_owned()),
            );
            assert_eq!(tool_scope(belt, &tools, browse), expected, "{belt:?}");
        }
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
    assert_eq!(
        belt_prompt(Belt::Learner, &inputs),
        None,
        "the learner speaks with the brief's instructions, per call"
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
    assert!(
        !context.browser_research,
        "the experiment is off by default"
    );
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

/// A fresh directory of this test process's own.
fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("teeny-specs-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_live_prompt_lives_under_the_workspace_prompts_by_agent_id() {
    let prompts = LivePrompts::new(Path::new("/w"), "teeny/inst");
    assert_eq!(prompts.relative(Belt::Reply), "teeny/inst/teeny-reply.md");
    assert_eq!(
        prompts.path(Belt::Learner),
        Path::new("/w/agent/prompts/teeny/inst/teeny-learner.md")
    );
}

#[test]
fn a_live_prompt_is_written_when_it_changes_or_goes_missing() {
    let dir = scratch("written");
    let prompts = LivePrompts::new(&dir, "teeny/inst");
    let path = prompts.path(Belt::Chat);
    prompts.write(Belt::Chat, "first").unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "first");
    prompts.write(Belt::Chat, "second").unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
    std::fs::remove_file(&path).unwrap();
    prompts.write(Belt::Chat, "second").unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "second",
        "a file pruned from under a live agent is written again"
    );
}

/// Two turns refreshing at once: the render and the write happen under one
/// lock, so the file ends as the later render saw the world, never an
/// earlier render written over a later one.
#[test]
fn a_refresh_renders_under_the_lock_so_an_older_render_never_lands_last() {
    let dir = scratch("ordered");
    let prompts = Arc::new(LivePrompts::new(&dir, "teeny/inst"));
    let brief = Arc::new(Mutex::new("old"));
    let (rendering, rendered) = std::sync::mpsc::channel::<()>();
    let (release, released) = std::sync::mpsc::channel::<()>();
    let slow = {
        let prompts = Arc::clone(&prompts);
        let brief = Arc::clone(&brief);
        std::thread::spawn(move || {
            prompts
                .refresh(Belt::Reply, || {
                    let text = brief.lock().unwrap().to_owned();
                    rendering.send(()).unwrap();
                    released.recv().unwrap();
                    text
                })
                .unwrap();
        })
    };
    rendered.recv().unwrap();
    *brief.lock().unwrap() = "new";
    let fast = {
        let prompts = Arc::clone(&prompts);
        let brief = Arc::clone(&brief);
        std::thread::spawn(move || {
            prompts
                .refresh(Belt::Reply, || brief.lock().unwrap().to_owned())
                .unwrap();
        })
    };
    std::thread::sleep(std::time::Duration::from_millis(50));
    release.send(()).unwrap();
    slow.join().unwrap();
    fast.join().unwrap();
    assert_eq!(
        std::fs::read_to_string(prompts.path(Belt::Reply)).unwrap(),
        "new"
    );
}

#[test]
fn stale_instance_prompts_are_pruned_and_live_ones_kept() {
    let dir = scratch("pruned");
    let prompts = LivePrompts::new(&dir, "teeny/mine");
    prompts.write(Belt::Reply, "mine").unwrap();
    let other = LivePrompts::new(&dir, "teeny/other");
    other.write(Belt::Reply, "theirs").unwrap();
    prompts.prune(std::time::Duration::from_secs(3600));
    assert!(other.path(Belt::Reply).exists(), "fresh, kept");
    prompts.prune(std::time::Duration::ZERO);
    assert!(!other.path(Belt::Reply).exists(), "stale, pruned");
    assert!(
        prompts.path(Belt::Reply).exists(),
        "our own is never pruned"
    );
}

#[test]
fn every_agent_keeps_its_own_turns_under_the_one_shared_memory_root() {
    let deps = Deps::fake();
    let mut vars = crate::deps::fake_vars();
    vars.push(("TEENY_MEMORY_ROOT", "team:teeny-shadow"));
    let context = SpecContext::from_config(&teeny_core::Config::from_vars(vars).unwrap());
    assert_eq!(context.memory_root, "team:teeny-shadow");
    let bound = |id: &str, spec: AgentSpec| {
        let expected = MemoryBinding::new(id).root("team:teeny-shadow");
        assert!(
            format!("{spec:?}").contains(&format!("{:?}", Some(&expected))),
            "{spec:?}"
        );
    };
    for belt in [Belt::Reply, Belt::Chart, Belt::Operator] {
        let tools = Arc::new(belt.build(&deps));
        bound(
            agent_id(belt),
            belt_spec(belt, tools, "prompt".into(), &context),
        );
    }
    // The fixed belts register on a live prompt file, the learner too.
    for belt in [Belt::Drafter, Belt::Learner] {
        let tools = Arc::new(belt.build(&deps));
        bound(
            agent_id(belt),
            live_spec(belt, tools, "teeny/x/prompt.md".into(), &context),
        );
    }
    // A room, the researcher included, under its persona's id.
    let researcher = room_id("researcher").unwrap();
    assert_eq!(researcher, "teeny-room-researcher");
    let tools = Arc::new(Belt::Room.build(&deps));
    bound(
        &researcher,
        spec_as(&researcher, Belt::Room, tools, "prompt".into(), &context),
    );
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
        for name in tool_scope(belt, &belt.build(&deps), true) {
            assert!(!refused(&name), "{belt:?}: {name}");
        }
    }
}
