//! The account's long-term memory: a CortexDB `/v1` client with the same
//! scope, labels and idempotency keys as the TypeScript service, so the
//! memories it already wrote are found again, plus the bodies of the
//! `recall_memory` and `remember` tools.
//!
//! [`Memory`] is the seam: [`CortexMemory`] speaks HTTP, [`InMemoryMemory`]
//! keeps everything in process for tests.

pub mod breaker;
mod cortex;
mod error;
pub mod experience;
mod fake;
pub mod health;
mod memory;
mod person;
pub mod text;
pub mod tools;

pub use breaker::{RecallBreaker, RecallPaused};
pub use cortex::{CortexMemory, CortexOptions, ForgetSelector, RECALL_LABELS, cap_pack};
pub use error::{MemoryError, Result};
pub use experience::{CAPTURE_VERSION, Content, Experience, Role};
pub use fake::InMemoryMemory;
pub use memory::{ABOUT_QUERY, Memory, format_recall};
pub use person::{ChatKind, MemoryChatMessage, MemoryPost, Person};
