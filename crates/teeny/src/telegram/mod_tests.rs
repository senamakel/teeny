use teeny_agent::deps::fake_vars;
use teeny_core::Config;

use super::*;

fn config(extra: &[(&'static str, &'static str)]) -> Config {
    let mut vars = fake_vars();
    vars.extend_from_slice(extra);
    Config::from_vars(vars).unwrap()
}

const TOKEN: (&str, &str) = ("TELEGRAM_BOT_TOKEN", "123456:ABCdefGHIjklMNOpqrSTUvwxYZ012345678");

#[test]
fn picks_teenys_own_poller_by_default() {
    assert_eq!(select(&config(&[TOKEN])), Selected::Teeny);
}

#[test]
fn picks_the_openhuman_channel_only_when_asked() {
    assert_eq!(
        select(&config(&[TOKEN, ("TELEGRAM_TRANSPORT", "openhuman")])),
        Selected::OpenHuman
    );
}

#[test]
fn runs_nothing_without_a_token_or_outside_the_ingest_role() {
    assert!(matches!(select(&config(&[])), Selected::Off(_)));
    assert!(matches!(
        select(&config(&[TOKEN, ("SERVICE_ROLE", "reply")])),
        Selected::Off(_)
    ));
    assert_eq!(
        select(&config(&[TOKEN, ("SERVICE_ROLE", "ingest")])),
        Selected::Teeny
    );
}
