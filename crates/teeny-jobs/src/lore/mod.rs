//! The lore post, on its days: the afternoon slot's fallback (dispatched by
//! the afternoon job) and the evening slot's (dispatched by
//! [`crate::evening`]). Two names, two guards and two journal ids, so an
//! afternoon page and an evening page on the same day are two turns.
//!
//! A port of `legacy/src/lore/*` and `legacy/src/jobs/lore-job.ts`.

pub mod job;
pub mod pipeline;

pub use job::{dry_run, run, run_afternoon, run_evening, run_pipeline};
pub use pipeline::{LorePipeline, LoreResult, LoreRoom, SeedSource};
