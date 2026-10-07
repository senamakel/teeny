//! Teeny's harnesses and publishing pipelines.
//!
//! Every pipeline talks to a model agent through [`runner::AgentRunner`], never
//! to OpenHuman directly. In production the runner is backed by embedded
//! OpenHuman agents (one per [`teeny_agent::posture::Belt`], plus one per
//! backrooms persona); in tests it is [`runner::ScriptedRunner`]. The
//! scheduler (OpenHuman cron system jobs) calls the `run` entry point of each
//! job module.

pub mod afternoon;
pub mod article;
pub mod backrooms;
pub mod brain_fold;
pub mod calendar;
pub mod chart;
pub mod chat;
pub mod ctx;
pub mod evening;
pub mod heat;
pub mod ingest;
pub mod journal_glue;
pub mod lore;
pub mod moon;
pub mod morning;
pub mod narrative;
pub mod quote;
pub mod reply;
pub mod runner;
