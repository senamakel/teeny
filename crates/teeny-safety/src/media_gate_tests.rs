//! Port of "the explicit-ask gate" in `legacy/test/capabilities.test.ts`.

use std::sync::Arc;

use serde_json::json;

use super::*;
use crate::model::{Message, Role, ScriptedModel};

#[tokio::test]
async fn asks_the_judge_and_answers_no_when_the_judge_cannot_be_reached() {
    let model = Arc::new(ScriptedModel::new([Ok(
        json!({ "image": true, "video": false }),
    )]));
    let gate = MediaGate::new(model.clone());
    assert_eq!(
        gate.asks("@teeny draw me a tiny on the moon").await,
        MediaAsk {
            image: true,
            video: false
        }
    );
    let request = &model.requests()[0];
    assert_eq!(request.messages[0].role, Role::System);
    assert_eq!(
        request.messages[1],
        Message::user("POST:\n@teeny draw me a tiny on the moon")
    );
    assert_eq!(request.temperature, 0.0);

    let down = MediaGate::new(Arc::new(ScriptedModel::new([Err("503".into())])));
    assert_eq!(down.asks("anything").await, MediaAsk::NONE);
    assert_eq!(gate.asks("   ").await, MediaAsk::NONE);
    assert_eq!(model.requests().len(), 1);
}

#[tokio::test]
async fn reads_a_loose_answer_the_way_the_legacy_gate_did() {
    let model = Arc::new(ScriptedModel::new([
        Ok(json!({ "image": "yes", "video": 0 })),
        Ok(json!("nonsense")),
    ]));
    let gate = MediaGate::new(model);
    assert_eq!(
        gate.asks("pic pls").await,
        MediaAsk {
            image: true,
            video: false
        }
    );
    assert_eq!(gate.asks("pic pls").await, MediaAsk::NONE);
}
