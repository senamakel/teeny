//! Port of the "recognising its own token" block of `legacy/test/thread-context.test.ts`.

use super::*;

const SOLANA: &str = "2AF7CqwieUjUPALL7icuZtL3X7wENdjUjGBMmfV2pump";
const ERC20: &str = "0xb9CE619b168f325b4eb8C2E8E073501838C7A407";
const OWN: &[&str] = &[SOLANA, ERC20];

#[test]
fn finds_the_contract_however_it_is_embedded_in_a_post() {
    let text = format!("@teenydroid maybe buy some more solana:{SOLANA} ? 👀");
    assert_eq!(find_own_token_addresses(&text, OWN), [SOLANA]);
}

#[test]
fn matches_the_erc20_regardless_of_checksum_casing() {
    let text = format!("send to {}", ERC20.to_lowercase());
    assert_eq!(find_own_token_addresses(&text, OWN), [ERC20]);
}

#[test]
fn says_nothing_about_a_contract_that_is_not_the_accounts_own() {
    assert!(find_own_token_addresses("buy 0x1234567890123456789012345678901234567890", OWN).is_empty());
}

#[test]
fn says_nothing_when_no_addresses_are_configured() {
    assert!(find_own_token_addresses(&format!("buy {SOLANA}"), &[] as &[&str]).is_empty());
}

// The exact correction that was needed: the reply called its own token
// somebody else's shill.
#[test]
fn tells_the_drafter_it_is_looking_at_its_own_token_not_a_shill() {
    let note = own_token_note(&[SOLANA.into()]).join("\n");
    assert!(note.contains("YOUR OWN $TINY contract address"));
    assert!(note.contains("not somebody shilling you a different coin"));
}

#[test]
fn adds_nothing_at_all_when_there_is_nothing_to_correct() {
    assert!(own_token_note(&[]).is_empty());
}
