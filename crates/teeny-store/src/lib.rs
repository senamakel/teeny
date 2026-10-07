//! Teeny's stores: the Mongo repositories (the mention queue, the X read
//! cache, the journal, the inference and post ledgers, the people log, the
//! safety holds, the Telegram transcript) and the file-state cursor.
//!
//! Every repository is a trait with a Mongo implementation and an in-memory
//! one, so the crates above can be tested without a database. Document field
//! names are the TypeScript service's, one for one: the production database
//! and the `web/` site read the same rows.

mod error;
pub mod file_state;
pub mod holds;
pub mod inference;
pub mod journal;
pub mod mentions;
pub mod mongo;
pub mod people;
pub mod posts;
mod support;
pub mod transcript;
pub mod x_cache;

pub use error::{Result, StoreError};
pub use file_state::{BotState, FileStateStore, InMemoryStateStore, StateStore};
pub use holds::{Hold, HoldLog, HoldStatus, InMemoryHoldLog, MongoHoldLog};
pub use inference::{
    InMemoryLedger, Inference, InferenceInput, InferenceLedger, MongoLedger, Tokens, Totals,
};
pub use journal::{InMemoryJournal, Journal, MongoJournal, Reflection, ReflectionInput};
pub use mentions::{
    InMemoryMentionQueue, MentionQueue, MentionStatus, MongoMentionQueue, QueuedMention,
    QueuedPost, RepliedFilter, Reply, ReplyLane, StatusCounts,
};
pub use mongo::connect;
pub use people::{InMemoryPeople, Interaction, MongoPeople, Observation, People, PersonRecord};
pub use posts::{
    Hot, InMemoryPostLedger, MetricSample, MongoPostLedger, NewOwnPost, OwnPost, PostLedger,
};
pub use support::{iso, iso_now};
pub use transcript::{
    ChatTranscript, ChatTurn, InMemoryChatTranscript, MongoChatTranscript, Speaker, TranscriptEntry,
};
pub use x_cache::{InMemoryXCache, MongoXCache, XCacheEntry, XCacheKind, XCacheStore, XCacheTtl};
