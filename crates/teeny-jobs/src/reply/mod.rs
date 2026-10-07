//! Answering mentions on X: the reply job, its drafter, and the worker loop
//! that drains the queue.
//!
//! Every reply is drafted by a public turn (`teeny-reply`, `Origin::Public`)
//! that holds no posting tool. Host code runs the content filter and the
//! house style on the draft, then posts it through a [`ReplyOutlet`]
//! (an [`XPublisher`](teeny_x::XPublisher) in production, `DRY_RUN`
//! honoured). [`run`] is one batch; [`worker::ReplyWorker`] loops it.

pub mod draft;
pub mod harness;
pub mod job;
pub mod outlet;
pub mod worker;

pub use draft::{ArticleLookup, ReplyArticle, ReplyDraft};
pub use job::{JOB, Pacing, gap_ms, run};
pub use outlet::{RecordingOutlet, ReplyOutlet, XOutlet};

#[cfg(test)]
pub mod test_support;
