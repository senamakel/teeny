//! A port of `legacy/test/reply-worker.test.ts`.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;

use super::*;

/// Answers from a script, then "nothing to do"; counts every call.
#[derive(Default)]
struct Script {
    answers: Mutex<VecDeque<anyhow::Result<bool>>>,
    calls: AtomicUsize,
    delay: Duration,
}

impl Script {
    fn of(answers: Vec<anyhow::Result<bool>>) -> Arc<Self> {
        Arc::new(Self {
            answers: Mutex::new(answers.into()),
            ..Self::default()
        })
    }
}

#[async_trait]
impl Batch for Script {
    async fn run_batch(&self) -> anyhow::Result<bool> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(self.delay).await;
        self.answers
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Ok(false))
    }
}

fn worker(script: &Arc<Script>, idle: u64, error: u64) -> ReplyWorker {
    ReplyWorker::new(
        script.clone(),
        Duration::from_millis(idle),
        Duration::from_millis(error),
    )
}

#[tokio::test]
async fn comes_straight_back_for_more_while_there_is_work_without_waiting() {
    let script = Script::of(vec![Ok(true), Ok(true), Ok(true)]);
    let running = worker(&script, 60_000, 60_000).start();
    tokio::time::sleep(Duration::from_millis(100)).await;
    running.stop().await;
    // Three batches of work, then the first idle look.
    assert_eq!(script.calls.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn waits_when_the_queue_is_empty_rather_than_spinning() {
    let script = Script::of(Vec::new());
    let running = worker(&script, 60_000, 60_000).start();
    tokio::time::sleep(Duration::from_millis(100)).await;
    running.stop().await;
    assert_eq!(script.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn backs_off_after_a_failure_instead_of_hammering() {
    let script = Script::of(vec![Err(anyhow::anyhow!("model down")), Ok(true)]);
    let running = worker(&script, 10, 60_000).start();
    tokio::time::sleep(Duration::from_millis(100)).await;
    running.stop().await;
    assert_eq!(script.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn lets_the_batch_in_flight_finish_before_it_stops() {
    let script = Arc::new(Script {
        answers: Mutex::new(VecDeque::from([Ok(true)])),
        delay: Duration::from_millis(80),
        ..Script::default()
    });
    let running = worker(&script, 60_000, 60_000).start();
    tokio::time::sleep(Duration::from_millis(20)).await;
    let stopping = std::time::Instant::now();
    running.stop().await;
    assert!(stopping.elapsed() >= Duration::from_millis(40));
    assert_eq!(script.calls.load(Ordering::SeqCst), 1);
}
