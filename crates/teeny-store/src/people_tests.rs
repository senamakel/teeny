use std::collections::HashMap;

use super::*;

fn seen(user_id: &str, kind: Interaction) -> Observation {
    Observation {
        user_id: user_id.into(),
        username: None,
        kind,
        at: None,
    }
}

/// The people log's contract, run against every implementation.
async fn contract(people: &dyn People) {
    people
        .observe(&Observation {
            username: Some("alice".into()),
            at: Some("2026-09-13T01:00:00.000Z".into()),
            ..seen("a", Interaction::Like)
        })
        .await
        .unwrap();
    people
        .observe(&Observation {
            at: Some("2026-09-13T02:00:00.000Z".into()),
            ..seen("a", Interaction::Reply)
        })
        .await
        .unwrap();
    people
        .observe(&seen("b", Interaction::Quote))
        .await
        .unwrap();
    people
        .observe(&seen("c", Interaction::Follow))
        .await
        .unwrap();

    let ids: Vec<String> = ["a", "b", "c", "nobody"].map(String::from).into();
    assert_eq!(
        people.scores(&ids).await.unwrap(),
        HashMap::from([
            (
                "a".to_owned(),
                Interaction::Like.weight() + Interaction::Reply.weight()
            ),
            ("b".to_owned(), Interaction::Quote.weight()),
            ("c".to_owned(), Interaction::Follow.weight()),
            ("nobody".to_owned(), 0),
        ])
    );
    assert!(people.scores(&[]).await.unwrap().is_empty());

    // The regulars rank first.
    let top = people.top(2).await.unwrap();
    assert_eq!(
        top.iter().map(|p| p.user_id.as_str()).collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert_eq!(top[0].username.as_deref(), Some("alice"));
    assert_eq!(
        top[0].counts,
        [(Interaction::Like, 1), (Interaction::Reply, 1)].into()
    );
    assert_eq!(top[0].first_seen_at, "2026-09-13T01:00:00.000Z");
    assert_eq!(top[0].last_seen_at, "2026-09-13T02:00:00.000Z");
}

#[tokio::test]
async fn scores_by_what_people_did_and_ranks_the_regulars_first() {
    contract(&InMemoryPeople::default()).await;
}

#[test]
fn writing_is_worth_more_than_a_tap() {
    use Interaction::*;
    let weights: Vec<_> = [Quote, Reply, Mention, Repost, Follow, Like, Answered]
        .iter()
        .map(|i| i.weight())
        .collect();
    assert_eq!(weights, [4, 3, 3, 2, 2, 1, 1]);
}

#[tokio::test]
#[ignore = "needs a Mongo at TEENY_TEST_MONGO_URI"]
async fn the_mongo_people_log_keeps_the_contract() {
    let Some(db) = crate::mongo::testing::database().await else {
        return;
    };
    let name = crate::mongo::testing::collection("people");
    contract(&MongoPeople::with_collection(&db, &name)).await;
    db.collection::<mongodb::bson::Document>(&name)
        .drop()
        .await
        .unwrap();
}
