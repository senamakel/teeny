//! The account's memory on the real runtime: the runtime boots with a
//! reference engine installed as its shared store, and what the host
//! captures through `shared_memory` lands at the shared root, where an
//! agent bound there reads it, and comes back on recall.
//!
//! One `#[test]`: a runtime is one per process, and its engine is
//! process-wide.

use std::sync::Arc;

use openhuman_embed::memory::ItemsQuery;
use teeny_agent::runtime::{self, HostEngine, MemorySettings, RuntimeSettings};
use teeny_agent::shared_memory::shared_memory;
use teeny_memory::{CortexOptions, Memory, MemoryPost, Person};
use tinymemory_api::conformance::ReferenceEngine;
use tinymemory_api::{ItemKind, ListRequest, MemoryEngine, MetaFilter};

const ROOT: &str = "team:teeny";

#[test]
fn the_hosts_captures_land_at_the_shared_root_and_are_recalled_from_it() {
    let engine = Arc::new(ReferenceEngine::new());
    let data_dir = std::env::temp_dir().join(format!("teeny-shared-memory-{}", std::process::id()));
    let settings = RuntimeSettings {
        data_dir: data_dir.clone(),
        chrome_path: None,
        unattended_actions: Vec::new(),
        allowed_domains: Vec::new(),
        tinycomputer_module: None,
        // Nothing leaves the machine.
        backend_url: Some("http://127.0.0.1:9".into()),
        memory: MemorySettings {
            engine: Some(HostEngine(engine.clone())),
            ..MemorySettings::default()
        },
    };
    let memory = shared_memory(settings.clone(), CortexOptions::default());

    runtime::executor().block_on(async move {
        // Built lazily: the first capture boots the runtime.
        assert!(runtime::existing().is_none());
        memory
            .capture_mention(&MemoryPost {
                id: "10".into(),
                text: "is the lab door open".into(),
                author_id: Some("7".into()),
                author_username: Some("ana".into()),
                conversation_id: Some("55".into()),
                created_at: Some("2026-09-12T23:00:00.000Z".into()),
            })
            .await
            .expect("the capture is stored");
        let rt = runtime::existing().expect("the capture booted the runtime");

        // On the engine, at the root, as the legacy client labelled it.
        let items = engine
            .list(ListRequest::new(MetaFilter::default(), 10))
            .await
            .unwrap()
            .items;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].meta.namespace.to_string(), ROOT);
        assert_eq!(items[0].meta.tags, ["x", "mention", "@ana", "thread:55"]);
        assert_eq!(items[0].meta.source.id.as_deref(), Some("x:mention:2:10"));

        // An agent bound to the root reads it: its node inherits the root.
        let facade = rt.memory(ROOT).expect("the root binds");
        assert!(facade.status().on);
        let reply_reads = facade
            .search(openhuman_embed::memory::SearchQuery {
                query: "door".into(),
                agent_id: Some("teeny-reply".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(reply_reads.hits.len(), 1);
        let conversations = facade
            .list(ItemsQuery {
                kinds: vec![ItemKind::Conversation],
                ..ItemsQuery::default()
            })
            .await
            .unwrap();
        assert_eq!(conversations.items.len(), 1);

        // And the host's own recall finds it, by topic and by person.
        let ana = Person::x("7", Some("ana"));
        let recalled = memory.recall("door", Some(&ana)).await.unwrap();
        assert!(recalled.contains("ABOUT @ana:"), "{recalled}");
        assert!(
            recalled.contains("@ana: is the lab door open"),
            "{recalled}"
        );

        // A second capture of the same mention is the same item.
        memory
            .capture_mention(&MemoryPost {
                id: "10".into(),
                text: "is the lab door open".into(),
                author_id: Some("7".into()),
                author_username: Some("ana".into()),
                conversation_id: Some("55".into()),
                created_at: Some("2026-09-12T23:00:00.000Z".into()),
            })
            .await
            .unwrap();
        let again = engine
            .list(ListRequest::new(MetaFilter::default(), 10))
            .await
            .unwrap()
            .items;
        assert_eq!(again.len(), 1);
    });
    let _ = std::fs::remove_dir_all(data_dir);
}
