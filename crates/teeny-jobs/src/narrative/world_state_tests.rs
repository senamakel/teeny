use chrono::TimeZone;

use super::*;

struct Fixed {
    fail: bool,
}

#[async_trait]
impl WorldSources for Fixed {
    async fn followers(&self) -> anyhow::Result<Option<u64>> {
        if self.fail {
            anyhow::bail!("x 401");
        }
        Ok(Some(8_500))
    }
    async fn mentions(&self) -> anyhow::Result<Vec<String>> {
        if self.fail {
            anyhow::bail!("x 401");
        }
        Ok(vec!["is teeny  going to\nget out".into(), "lol".into()])
    }
    async fn last_episode_metrics(&self, _post_id: &str) -> anyhow::Result<Option<(u64, u64)>> {
        Ok(Some((40, 7)))
    }
}

#[tokio::test]
async fn reads_every_source_and_renders_only_what_is_true() {
    let state = build_world_state(
        &Fixed { fail: false },
        4,
        Some((3, Some("the inch".into()), Some("one inch out".into()), Some("8".into()))),
        Utc.with_ymd_and_hms(2026, 9, 21, 3, 0, 0).unwrap(),
    )
    .await;
    assert_eq!(state.mentions, ["is teeny going to get out", "lol"]);
    assert!(state.problems.is_empty());
    let fields = trigger_fields(&state);
    assert_eq!(fields["hourUtc"], Field::Number(3.0));
    assert_eq!(fields["lastLikes"], Field::Number(40.0));
    assert_eq!(fields["mentions24h"], Field::Number(2.0));
    let text = render_world_state(&state, "teenydroid");
    assert!(text.contains("03:00 utc (the small hours; senamakel is asleep). episode 4."));
    assert!(text.contains("@teenydroid has 8500 followers on x. the last episode did 40 likes and 7 replies."));
    assert!(text.contains("2 people talked about teeny today. some of what they said"));
    assert!(text.contains("last episode: #3 \"the inch\" -- one inch out"));
    assert!(!text.contains('$'));
}

#[tokio::test]
async fn renders_unknown_never_a_guess_for_every_source_that_fails() {
    let state = build_world_state(&Fixed { fail: true }, 1, None, Utc.with_ymd_and_hms(2026, 9, 21, 19, 0, 0).unwrap()).await;
    assert!(state.followers.is_none());
    assert_eq!(state.problems, ["followers: x 401", "mentions: x 401"]);
    let text = render_world_state(&state, "teenydroid");
    assert!(text.contains("(evening; senamakel is making tea). episode 1."));
    assert!(text.contains("an unknown number of followers"));
    assert!(text.contains("0 people talked about teeny today."));
    assert!(text.contains("last episode: none. this is the first."));
}
