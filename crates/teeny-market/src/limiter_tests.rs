use std::sync::{Arc, Mutex};

use super::*;

#[tokio::test]
async fn runs_one_call_at_a_time_in_order() {
    let limit = Limiter::new(Duration::ZERO);
    let order = Arc::new(Mutex::new(Vec::new()));
    let slow = |n: u32, ms: u64| {
        let order = order.clone();
        async move {
            tokio::time::sleep(Duration::from_millis(ms)).await;
            order.lock().unwrap().push(n);
        }
    };
    tokio::join!(
        limit.run(slow(1, 20)),
        limit.run(slow(2, 1)),
        limit.run(slow(3, 1))
    );
    assert_eq!(*order.lock().unwrap(), vec![1, 2, 3]);
}

#[tokio::test(start_paused = true)]
async fn keeps_a_gap_between_calls() {
    let limit = Limiter::new(Duration::from_secs(1));
    let started = Arc::new(Mutex::new(Vec::new()));
    let work = || {
        let started = started.clone();
        async move { started.lock().unwrap().push(Instant::now()) }
    };
    tokio::join!(limit.run(work()), limit.run(work()), limit.run(work()));
    let started = started.lock().unwrap();
    assert_eq!(started.len(), 3);
    assert!(started[1] - started[0] >= Duration::from_secs(1));
    assert!(started[2] - started[1] >= Duration::from_secs(1));
}

/// One rejection must not wedge everything queued behind it.
#[tokio::test]
async fn keeps_going_after_a_call_fails() {
    let limit = Limiter::new(Duration::ZERO);
    let failed: Result<(), &str> = limit.run(async { Err("429") }).await;
    assert_eq!(failed, Err("429"));
    assert_eq!(limit.run(async { "ok" }).await, "ok");
}

#[tokio::test]
async fn surfaces_the_result_and_the_error_to_the_right_caller() {
    let limit = Limiter::new(Duration::ZERO);
    type Answer = Result<&'static str, &'static str>;
    let results: (Answer, Answer, Answer) = tokio::join!(
        limit.run(async { Ok("first") }),
        limit.run(async { Err("second failed") }),
        limit.run(async { Ok("third") }),
    );
    assert_eq!(results, (Ok("first"), Err("second failed"), Ok("third")));
}
