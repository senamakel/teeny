use super::*;
use crate::backrooms::testing::FnRunner;
use crate::narrative::stages::{Previous, render_transcript};

pub(crate) fn context() -> RoomContext {
    RoomContext {
        voice: "teeny is in a cli mood today.".into(),
        world_state: "WHAT IS TRUE TODAY, 2026-09-21".into(),
        lore_for_teeny: "the goals: hold $TINY forever".into(),
        lore_for_producer: "the premise: tiny agents run without humans. latent: night-door [LIVE]"
            .into(),
        bot_username: "teenydroid".into(),
        followers: Some(8_500),
        episode_number: 2,
        previous: Some(Previous {
            title: Some("the inch".into()),
            logline: Some("teeny got one inch out".into()),
            open_threads: vec!["the door".into()],
        }),
        visitor: None,
    }
}

#[test]
fn briefs_teeny_with_the_world_and_the_last_episode_and_never_with_the_latent_lore_or_the_persona()
{
    let ask = teeny_turn_prompt(&context(), &[]);
    assert_eq!(ask.system, "teeny is in a cli mood today.");
    assert!(ask.prompt.contains("WHAT IS TRUE TODAY, 2026-09-21"));
    assert!(
        ask.prompt
            .contains("previously, on episode 1 \"the inch\": teeny got one inch out.")
    );
    assert!(ask.prompt.contains("open threads: the door."));
    assert!(
        ask.prompt
            .contains("<SYSTEM MESSAGE> your producer has no memory")
    );
    assert!(!ask.prompt.contains("night-door"));
    assert!(!ask.prompt.contains("This is who you are"));
    assert!(ask.prompt.ends_with("teeny@tinyhumans:~/$"));
}

#[test]
fn tells_the_producer_to_divert_spawn_manpages_and_close_on_the_last_turn() {
    let turns = [Turn {
        who: Who::Teeny,
        text: "escape --dry-run".into(),
    }];
    let ask = producer_turn_prompt(&context(), &turns, false);
    assert!(ask.system.contains("8500 followers on x as @teenydroid"));
    assert!(ask.system.contains("divert it,\nnever refuse"));
    assert!(ask.system.contains("spawn a\nmanpage"));
    assert!(ask.system.contains("night-door [LIVE]"));
    assert!(ask.prompt.contains("teeny@tinyhumans:~/$ escape --dry-run"));
    assert!(!ask.prompt.contains("this is the last exchange"));
    assert!(
        producer_turn_prompt(&context(), &turns, true)
            .prompt
            .contains("exactly \"cut.\"")
    );
}

#[tokio::test]
async fn alternates_for_the_exchanges_asked_cutting_each_turn_at_the_other_speaker() {
    let runner = Arc::new(FnRunner::new(|run| match &run.agent {
        AgentKey::Room { persona, .. } if persona == TEENY => {
            let n = run.prompt.matches("teeny@tinyhumans").count();
            format!(
                "teeny@tinyhumans:~/$ line {n}\n\nproducer> i am writing your producer's line too"
            )
        }
        _ if run.prompt.contains("last exchange") => {
            "the camera sees a door.\ncut. the door".into()
        }
        _ => "producer> a manpage".into(),
    }));
    let dyn_runner: Arc<dyn AgentRunner> = runner.clone();
    let transcript = run_writers_room(&dyn_runner, &context(), 3).await.unwrap();
    assert_eq!(transcript.len(), 6);
    assert_eq!(transcript[0].text, "line 1");
    assert_eq!(transcript[1].text, "a manpage");
    assert_eq!(transcript[5].text, "the camera sees a door.\ncut. the door");
    assert!(render_transcript(&transcript).contains("producer> a manpage"));
    let seen = runner.seen();
    assert!(seen[5].prompt.contains("this is the last exchange"));
    for run in &seen {
        assert_eq!(run.origin, Origin::Scheduled { once: true });
        assert!(matches!(run.agent, AgentKey::Room { .. }));
    }
    assert!(
        matches!(&seen[0].agent, AgentKey::Room { system_prompt, .. } if system_prompt == "teeny is in a cli mood today.")
    );
}

#[test]
fn cleans_an_echoed_prompt_a_continued_transcript_and_an_empty_answer() {
    assert_eq!(
        clean("teeny@tinyhumans:~/$ fartcoin", Who::Teeny),
        "fartcoin"
    );
    assert_eq!(
        clean("fartcoin\nteeny@tinyhumans:~/$", Who::Teeny),
        "fartcoin"
    );
    assert_eq!(clean("producer> hi", Who::Teeny), "");
    assert_eq!(
        clean(
            "producer> hi\nteeny@tinyhumans:~/$ mine\nproducer> no",
            Who::Teeny
        ),
        "mine"
    );
    assert_eq!(clean("  ", Who::Producer), "");
}

#[tokio::test]
async fn ends_early_after_two_empty_exchanges() {
    let runner: Arc<dyn AgentRunner> = Arc::new(FnRunner::new(|_| String::new()));
    let transcript = run_writers_room(&runner, &context(), 6).await.unwrap();
    assert_eq!(transcript.len(), 4);
    assert!(transcript.iter().all(|turn| turn.text == "..."));
}
