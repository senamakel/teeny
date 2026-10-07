//! Teeny's harnesses and publishing pipelines.
//!
//! Every pipeline talks to a model agent through [`runner::AgentRunner`], never
//! to OpenHuman directly. In production the runner is backed by embedded
//! OpenHuman agents (one per [`teeny_agent::posture::Belt`], plus one per
//! backrooms persona); in tests it is [`runner::ScriptedRunner`]. The
//! scheduler (OpenHuman cron system jobs) calls the `run` entry point of each
//! job module.

pub mod runner;
pub mod ctx;
pub mod journal_glue;
pub mod morning;
pub mod backrooms;
pub mod brain_fold;
pub mod ingest;
pub mod reply;
pub mod heat;
pub mod chat;
pub mod narrative;
pub mod moon;
