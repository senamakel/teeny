//! The runtime's posture as data: the families it registers, the groups it
//! turns off, the services it starts and the config every agent starts from.
//! Building the runtime itself is exercised by `tests/posture.rs`, which owns
//! the process's one runtime.

use std::path::{Path, PathBuf};

use openhuman_embed::GroupMode;

use super::*;

fn settings() -> RuntimeSettings {
    RuntimeSettings {
        data_dir: PathBuf::from("/data"),
        chrome_path: Some("/usr/bin/chromium".into()),
        unattended_actions: vec!["click".into(), "press".into()],
        allowed_domains: vec!["dexscreener.com".into()],
        tinycomputer_module: Some(PathBuf::from("/opt/tinycomputer/libtinycomputer.so")),
        backend_url: None,
        memory: MemorySettings::default(),
    }
}

#[test]
fn the_workspace_lives_on_the_data_volume() {
    assert_eq!(
        settings().workspace_dir(),
        Path::new("/data/openhuman/workspace")
    );
}

#[test]
fn settings_come_from_the_teeny_config_with_an_absolute_data_dir() {
    let mut vars = crate::deps::fake_vars();
    vars.extend([
        ("TEENY_DATA", "relative-data"),
        ("BROWSE_ALLOWED_DOMAINS", "coingecko.com"),
        ("TINYCOMPUTER_MODULE_PATH", "/opt/tc.so"),
    ]);
    let config = teeny_core::Config::from_vars(vars).unwrap();
    let settings = RuntimeSettings::from_config(&config.openhuman);
    assert!(settings.data_dir.is_absolute());
    assert!(settings.data_dir.ends_with("relative-data"));
    assert_eq!(settings.allowed_domains, ["coingecko.com"]);
    assert_eq!(settings.unattended_actions, ["click", "press"]);
    assert_eq!(
        settings.tinycomputer_module.as_deref(),
        Some(Path::new("/opt/tc.so"))
    );
    assert_eq!(settings.backend_url, None);
}

#[test]
fn registers_skills_and_modules_and_none_of_the_risky_families() {
    let domains = domains();
    assert!(
        domains.skills && domains.modules,
        "skills, and the browser module"
    );
    assert!(
        domains.agent && domains.inference,
        "agents need to run turns"
    );
    assert!(domains.automation, "the cron scheduler");
    assert!(domains.platform, "the browser is filed under platform");
    for (family, on) in [
        ("mcp", domains.mcp),
        ("web3", domains.web3),
        ("desktop", domains.desktop),
        ("hosted", domains.hosted),
        ("flows", domains.flows),
        ("integrations", domains.integrations),
        ("runtimes", domains.runtimes),
        ("voice", domains.voice),
        ("media", domains.media),
        ("channels", domains.channels),
        ("operator", domains.operator),
    ] {
        assert!(!on, "{family} is registered");
    }
}

#[test]
fn no_agent_schedules_and_only_a_browsing_agent_keeps_platform() {
    for browses in [false, true] {
        let agent = agent_domains(browses);
        assert!(!agent.automation, "the cron tools are automation's");
        assert_eq!(agent.platform, browses);
        assert!(agent.skills && agent.modules && agent.inference);
        assert!(!agent.mcp && !agent.flows && !agent.web3 && !agent.channels);
    }
}

#[test]
fn every_tool_group_is_off() {
    let groups = tool_groups();
    for id in ToolGroups::ids() {
        assert_eq!(groups.mode(id), GroupMode::Off, "{id}");
    }
}

#[test]
fn starts_the_cron_scheduler_and_nothing_that_listens() {
    let services = services();
    assert!(services.cron);
    assert!(!services.rpc_http && !services.socketio, "no transport");
    assert!(!services.channels && !services.mcp_boot && !services.integrations);
    assert!(!services.update_scheduler && !services.login_gated);
}

#[test]
fn the_overlay_pins_native_tools_autonomy_and_cron() {
    let config = openhuman_config(&settings());
    assert_eq!(config.agent.tool_dispatcher, "native");
    assert!(
        config.autonomy.enabled,
        "access tiers do nothing without it"
    );
    assert!(config.cron.enabled);
    assert!(
        !config.memory.conversations.enabled && !config.memory.recall.enabled,
        "Teeny's memory is CortexDB, through its own tools"
    );
}

#[test]
fn the_overlay_configures_a_persistent_headless_browser_on_the_volume() {
    let browser = openhuman_config(&settings()).browser;
    assert!(browser.enabled && browser.headless);
    assert_eq!(browser.profile_mode, "persistent");
    assert_eq!(
        browser.profile_path.as_deref(),
        Some("/data/chrome-profile")
    );
    assert_eq!(
        browser.download_dir.as_deref(),
        Some("/data/media/downloads")
    );
    assert_eq!(browser.chrome_path.as_deref(), Some("/usr/bin/chromium"));
    assert_eq!(browser.unattended_actions, ["click", "press"]);
}

#[test]
fn unattended_actions_keep_known_kinds_only() {
    let mut settings = settings();
    settings.unattended_actions = vec![
        " Click ".into(),
        "scroll".into(),
        "fill".into(),
        "press".into(),
        "teleport".into(),
    ];
    let browser = openhuman_config(&settings).browser;
    // `scroll` is not a gated action kind (it never needed approval), so it
    // has nothing to allow and is dropped with the typo.
    assert_eq!(browser.unattended_actions, ["click", "fill", "press"]);
}

#[test]
fn the_overlay_loads_tinycomputer_from_the_local_build_when_set() {
    let config = openhuman_config(&settings());
    assert_eq!(config.modules.overrides.len(), 1);
    assert_eq!(config.modules.overrides[0].id, "tinycomputer");
    assert_eq!(
        config.modules.overrides[0].path,
        "/opt/tinycomputer/libtinycomputer.so"
    );

    let mut unset = settings();
    unset.tinycomputer_module = None;
    assert!(openhuman_config(&unset).modules.overrides.is_empty());
}

#[test]
fn x_and_twitter_are_never_browsable() {
    let listed: Vec<String> = [
        "dexscreener.com",
        "x.com",
        "X.COM",
        "https://twitter.com/teenydroid",
        "mobile.twitter.com",
        "*.x.com",
        "api.x.com",
        "t.co",
        "*",
        "com",
        "coingecko.com",
        "",
    ]
    .map(str::to_owned)
    .into();
    assert_eq!(
        browsable_domains(&listed),
        ["dexscreener.com", "coingecko.com"]
    );
    let mut settings = settings();
    settings.allowed_domains = listed;
    let allowed = openhuman_config(&settings).http_request.allowed_domains;
    assert_eq!(allowed, ["dexscreener.com", "coingecko.com"]);
}

#[test]
fn an_empty_list_browses_nothing_rather_than_everything() {
    let mut settings = settings();
    settings.allowed_domains.clear();
    assert!(
        openhuman_config(&settings)
            .http_request
            .allowed_domains
            .is_empty(),
        "OpenHuman's default is [\"*\"], which includes X"
    );
}

fn with_store() -> RuntimeSettings {
    RuntimeSettings {
        memory: MemorySettings {
            store: Some(MemoryStore {
                url: "http://teeny-cortexdb:3141".into(),
                token: Secret::new("key"),
            }),
            ..MemorySettings::default()
        },
        ..settings()
    }
}

#[tokio::test]
async fn without_a_store_memory_is_off_and_no_turn_reads_or_logs_it() {
    let config = openhuman_config(&settings());
    assert_eq!(config.memory.engine, "none");
    assert!(!config.memory.conversations.enabled);
    assert!(!config.memory.recall.enabled);
    assert!(memory_engine(&settings().memory).await.unwrap().is_none());
}

#[test]
fn with_a_store_every_turn_reads_the_shared_root_and_is_logged_under_it() {
    let config = openhuman_config(&with_store());
    assert_ne!(config.memory.engine, "none");
    assert!(config.memory.conversations.enabled);
    assert!(config.memory.recall.enabled);
    assert_eq!(config.memory.root.as_deref(), Some("team:teeny"));

    let mut quiet = with_store();
    quiet.memory.turn_pack = false;
    let config = openhuman_config(&quiet);
    assert!(!config.memory.conversations.enabled && !config.memory.recall.enabled);
    assert_ne!(
        config.memory.engine, "none",
        "the store stays on for the host"
    );
}

#[tokio::test]
async fn the_store_is_installed_as_a_cortexdb_engine_and_a_tests_engine_wins() {
    // On the stack's network by name, which the engine reaches by relay.
    let engine = memory_engine(&with_store().memory).await.unwrap().unwrap();
    assert_eq!(engine.descriptor().id, "cortexdb");

    let reference: Arc<dyn MemoryEngine> =
        Arc::new(tinymemory_api::conformance::ReferenceEngine::new());
    let mut settings = with_store();
    settings.memory.engine = Some(HostEngine(Arc::clone(&reference)));
    let installed = memory_engine(&settings.memory).await.unwrap().unwrap();
    assert!(Arc::ptr_eq(&installed, &reference));
}

#[test]
fn the_learning_policy_and_the_store_come_from_the_teeny_config() {
    let mut vars = crate::deps::fake_vars();
    vars.extend([
        ("TEENY_MEMORY_URL", "http://teeny-cortexdb:3141"),
        ("TEENY_MEMORY_TOKEN", "key"),
        ("TEENY_MEMORY_ROOT", "team:teeny-shadow"),
        ("MEMORY_BUILD_BELIEFS_EVERY", "25"),
        ("MEMORY_LEARNINGS_LIMIT", "4"),
    ]);
    let config = teeny_core::Config::from_vars(vars).unwrap();
    let settings = RuntimeSettings::from_teeny(&config);
    assert_eq!(
        settings.data_dir,
        RuntimeSettings::from_config(&config.openhuman).data_dir
    );
    let memory = &settings.memory;
    assert_eq!(
        memory.store,
        Some(MemoryStore {
            url: "http://teeny-cortexdb:3141".into(),
            token: Secret::new("key"),
        })
    );
    assert_eq!(memory.root, "team:teeny-shadow");
    let learning = memory.learning();
    assert_eq!(
        (learning.build_beliefs_every, learning.learnings_limit),
        (25, 4)
    );
    let off = RuntimeSettings::from_teeny(
        &teeny_core::Config::from_vars(crate::deps::fake_vars()).unwrap(),
    );
    assert!(!off.memory.is_on());
}
