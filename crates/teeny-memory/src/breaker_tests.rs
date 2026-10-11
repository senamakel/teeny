use std::time::Duration;

use super::*;

#[test]
fn lets_calls_through_until_a_timeout_trips_it_for_the_cooldown() {
    let breaker = RecallBreaker::new(Duration::from_secs(60), 2);
    drop(breaker.enter().unwrap());
    breaker.trip();
    let refused = breaker.enter().unwrap_err();
    assert!(
        refused.to_string().starts_with("recall paused for "),
        "{refused}"
    );
    assert!(
        refused.to_string().ends_with("s after a timeout"),
        "{refused}"
    );
}

#[test]
fn a_zero_cooldown_never_pauses() {
    let breaker = RecallBreaker::new(Duration::ZERO, 2);
    breaker.trip();
    assert!(breaker.enter().is_ok());
}

#[test]
fn fails_fast_past_the_in_flight_limit_and_frees_a_place_when_a_call_ends() {
    let breaker = RecallBreaker::new(Duration::from_secs(60), 1);
    let first = breaker.enter().unwrap();
    assert_eq!(
        breaker.enter().unwrap_err().to_string(),
        "too many recalls in flight"
    );
    drop(first);
    assert!(breaker.enter().is_ok());
}

#[test]
fn is_shared_by_its_clones() {
    let breaker = RecallBreaker::new(Duration::from_secs(60), 2);
    breaker.clone().trip();
    assert!(breaker.enter().is_err());
}
