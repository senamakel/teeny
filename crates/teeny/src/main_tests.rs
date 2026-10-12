use super::*;

#[test]
fn the_runtime_runs_futures_on_worker_threads() {
    let runtime = build_runtime().unwrap();
    let answer = runtime.block_on(async { tokio::spawn(async { 42 }).await.unwrap() });
    assert_eq!(answer, 42);
}
