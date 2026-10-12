//! The serial: an episode on its evening days, written in writers' rooms
//! (each writer its own Room persona behind the runner), cut, scored, drawn,
//! filmed, captioned in teeny's own words and posted to X and Telegram,
//! with `narrative/LORE.md`, `VOICE.md`, the world state and a visitor's
//! likeness. A port of `legacy/src/narrative/*` and the narrative job.

pub mod job;
pub mod likeness;
pub mod picture;
pub mod pipeline;
pub mod room;
pub mod stages;
pub mod state;
pub mod telegram;
pub mod voice;
pub mod world_state;

pub use job::{dry_run, run, run_pipeline};
pub use pipeline::{Attached, Branch, NarrativePipeline, NarrativeResult, VisitorAs};
