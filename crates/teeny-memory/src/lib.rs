//! The account's long-term memory, plus the bodies of the `recall_memory`
//! and `remember` tools.
//!
//! [`Memory`] is the seam every caller holds. [`TinyMemory`] is the memory
//! the account runs on: the TinyMemory store every Teeny agent shares
//! through OpenHuman, behind the [`ItemStore`] seam. [`CortexMemory`] is the
//! CortexDB `/v1` client the TypeScript service wrote through, with the same
//! scope, labels and idempotency keys; it is what [`import`] reads the old
//! memory out of. [`InMemoryMemory`] keeps everything in process for tests.

pub mod breaker;
mod cortex;
mod error;
pub mod experience;
mod fake;
pub mod health;
pub mod import;
mod memory;
mod person;
pub mod text;
pub mod tiny;
pub mod tools;

pub use breaker::{RecallBreaker, RecallPaused};
pub use cortex::{
    CortexMemory, CortexOptions, EventsPage, ForgetSelector, RECALL_LABELS, cap_pack,
};
pub use error::{MemoryError, Result};
pub use experience::{CAPTURE_VERSION, Content, Experience, Role};
pub use fake::InMemoryMemory;
pub use memory::{ABOUT_QUERY, Memory, format_recall};
pub use person::{ChatKind, MemoryChatMessage, MemoryPost, Person};
pub use tiny::{DEFAULT_ROOT, EngineStore, ItemStore, TinyMemory, item_of};
