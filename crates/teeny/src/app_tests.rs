use teeny_agent::deps::fake_vars;

use super::*;

#[test]
fn keeps_the_poll_cursor_where_the_entrypoint_migrates_it() {
    let mut vars = fake_vars();
    vars.push(("TEENY_DATA", "/data"));
    let config = Config::from_vars(vars).unwrap();
    assert_eq!(cursor_path(&config), PathBuf::from("/data/state/state.json"));
}

/// Builds the whole service against the local Mongo (lazily: nothing is
/// read) and boots OpenHuman on a temp workspace.
#[test]
#[ignore = "boots the OpenHuman runtime; run from the repository root files"]
fn builds_every_part_from_the_configuration() {
    let executor = teeny_agent::runtime::executor();
    executor
        .block_on(executor.spawn(async {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
            let data = std::env::temp_dir().join(format!("teeny-app-{}", std::process::id()));
            let mut vars = fake_vars();
            let (soul, context, skills) = (
                root.join("SOUL.md").display().to_string(),
                root.join("CONTEXT.md").display().to_string(),
                root.join("skills").display().to_string(),
            );
            let data_text = data.display().to_string();
            let owned: Vec<(&'static str, String)> = vec![
                ("SOUL_PATH", soul),
                ("CONTEXT_PATH", context),
                ("SKILLS_DIR", skills),
                ("TEENY_DATA", data_text),
                ("MONGO_URI", "mongodb://127.0.0.1:27017".into()),
            ];
            let leaked: Vec<(&'static str, &'static str)> = owned
                .into_iter()
                .map(|(key, value)| (key, &*Box::leak(value.into_boxed_str())))
                .collect();
            vars.extend(leaked);
            let app = App::build(Config::from_vars(vars).unwrap()).await.unwrap();
            assert!(app.ctx.conversation.is_some());
            assert!(app.ctx.conversation().unwrap().articles.is_some());
            assert!(app.runtime.agent_ids().contains(&"teeny-operator".to_owned()));
            let _ = app.intake();
            std::fs::remove_dir_all(data).ok();
        }))
        .unwrap();
}
