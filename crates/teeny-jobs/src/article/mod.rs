//! The long piece: a backrooms room read down to one thought, planned,
//! written section by section, read for flow, proof-read with the humanizer
//! skill, gated, critiqued, drawn, then published as an X Article
//! (`ARTICLE_PUBLISH`; otherwise a rehearsal) and mirrored to the article
//! store that `/articles` serves. A port of `legacy/src/article/*`.

pub mod content_state;
pub mod job;
pub mod pipeline;
pub mod proofread;
pub mod publish;
pub mod stages;
pub mod store;

pub use job::{dry_run, run, run_pipeline};
pub use pipeline::{ArticlePipeline, ArticleResult};
pub use publish::publish_saved;
pub use store::{ArticleRecord, ArticleStatus, FileArticleStore};
