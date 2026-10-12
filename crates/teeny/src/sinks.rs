//! The intake's sinks over the real stores: memory from teeny-memory, the
//! people record from teeny-store.

use std::sync::Arc;

use async_trait::async_trait;
use teeny_store::{Interaction, Observation};
use teeny_x::activity::{ActivityEntry, ActivityInteraction, InteractionKind};
use teeny_x::intake::{ActivityMemory, IntakeError, PeopleSink};

/// Long-term memory, keeping the non-post activity.
pub struct MemorySink(pub Arc<dyn teeny_memory::Memory>);

#[async_trait]
impl ActivityMemory for MemorySink {
    async fn capture_activity(&self, entry: ActivityEntry) -> Result<(), IntakeError> {
        Ok(self
            .0
            .capture_activity(&entry.kind, &entry.id, entry.payload, None)
            .await?)
    }
}

/// The people record.
pub struct PeopleRecord(pub Arc<dyn teeny_store::People>);

#[async_trait]
impl PeopleSink for PeopleRecord {
    async fn observe(&self, interaction: ActivityInteraction) -> Result<(), IntakeError> {
        let kind = match interaction.kind {
            InteractionKind::Mention => Interaction::Mention,
            InteractionKind::Reply => Interaction::Reply,
            InteractionKind::Quote => Interaction::Quote,
            InteractionKind::Repost => Interaction::Repost,
            InteractionKind::Follow => Interaction::Follow,
            InteractionKind::Like => Interaction::Like,
        };
        let observation = Observation {
            user_id: interaction.user_id.as_str().to_owned(),
            username: interaction.username,
            kind,
            at: None,
        };
        Ok(self.0.observe(&observation).await?)
    }
}

#[cfg(test)]
#[path = "sinks_tests.rs"]
mod tests;
