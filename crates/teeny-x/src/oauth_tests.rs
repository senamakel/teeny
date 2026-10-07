use super::*;

/// The worked example from X's "Creating a signature" guide.
fn twitter_example() -> OAuth1 {
    OAuth1::new(
        "xvz1evFS4wEEPTGEFPHBog",
        Secret::new("kAcSOqF21Fu85e7zjz7ZN2U4ZRhfV3WpwPAoE3Z7kBw"),
        "370773112-GmHxMAgYyLbNEtIKZeRNFsMKPR9EyMZeS9weJAEb",
        Secret::new("LswwdoUaIvS8ltyTt5jkRh4J50vUPVVHtR2YPi5kE"),
    )
}

const URL: &str = "https://api.twitter.com/1.1/statuses/update.json?include_entities=true";
const STATUS: &str = "Hello Ladies + Gentlemen, a signed OAuth request!";
const NONCE: &str = "kYjzVBB8Y0ZFabxSWbWovY3uYSQ2pTgmZeNu2VS4cg";
const TIMESTAMP: u64 = 1_318_622_958;

#[test]
fn percent_encodes_everything_but_the_unreserved_set() {
    assert_eq!(
        percent_encode("Ladies + Gentlemen"),
        "Ladies%20%2B%20Gentlemen"
    );
    assert_eq!(
        percent_encode("An encoded string!"),
        "An%20encoded%20string%21"
    );
    assert_eq!(
        percent_encode("Dogs, Cats & Mice"),
        "Dogs%2C%20Cats%20%26%20Mice"
    );
    assert_eq!(percent_encode("☃"), "%E2%98%83");
    assert_eq!(percent_encode("!*'()"), "%21%2A%27%28%29");
    assert_eq!(percent_encode("az-AZ_09.~"), "az-AZ_09.~");
}

#[test]
fn builds_the_published_signature_base_string() {
    let base = twitter_example().base_string("post", URL, &[("status", STATUS)], NONCE, TIMESTAMP);
    assert_eq!(
        base,
        "POST&https%3A%2F%2Fapi.twitter.com%2F1.1%2Fstatuses%2Fupdate.json&include_entities%3Dtrue%26oauth_consumer_key%3Dxvz1evFS4wEEPTGEFPHBog%26oauth_nonce%3DkYjzVBB8Y0ZFabxSWbWovY3uYSQ2pTgmZeNu2VS4cg%26oauth_signature_method%3DHMAC-SHA1%26oauth_timestamp%3D1318622958%26oauth_token%3D370773112-GmHxMAgYyLbNEtIKZeRNFsMKPR9EyMZeS9weJAEb%26oauth_version%3D1.0%26status%3DHello%2520Ladies%2520%252B%2520Gentlemen%252C%2520a%2520signed%2520OAuth%2520request%2521"
    );
}

#[test]
fn produces_the_published_signature() {
    let header =
        twitter_example().header_with("POST", URL, &[("status", STATUS)], NONCE, TIMESTAMP);
    assert!(
        header.contains(r#"oauth_signature="hCtSmYh%2BiHYCEqBWrE7C7hYmtUk%3D""#),
        "{header}"
    );
    assert!(
        header.starts_with("OAuth oauth_consumer_key=\"xvz1evFS4wEEPTGEFPHBog\", oauth_nonce=")
    );
    assert!(header.contains(r#"oauth_token="370773112-GmHxMAgYyLbNEtIKZeRNFsMKPR9EyMZeS9weJAEb""#));
    assert!(header.contains(r#"oauth_version="1.0""#));
}

#[test]
fn signs_the_decoded_query_so_an_encoded_value_is_not_encoded_twice() {
    let signer = twitter_example();
    let encoded = signer.base_string(
        "GET",
        "https://api.x.com/2/tweets/search/recent?query=%40teeny%20-is%3Aretweet",
        &[],
        NONCE,
        TIMESTAMP,
    );
    assert!(
        encoded.contains("query%3D%2540teeny%2520-is%253Aretweet"),
        "{encoded}"
    );
    assert!(encoded.starts_with("GET&https%3A%2F%2Fapi.x.com%2F2%2Ftweets%2Fsearch%2Frecent&"));
}

#[test]
fn fresh_headers_carry_fresh_nonces() {
    let signer = twitter_example();
    let a = signer.header("GET", "https://api.x.com/2/users/me", &[]);
    let b = signer.header("GET", "https://api.x.com/2/users/me", &[]);
    assert_ne!(a, b);
}

#[test]
fn never_prints_its_secrets() {
    let printed = format!("{:?}", twitter_example());
    assert!(!printed.contains("kAcSOqF21Fu85e7zjz7ZN2U4ZRhfV3WpwPAoE3Z7kBw"));
    assert!(!printed.contains("LswwdoUaIvS8ltyTt5jkRh4J50vUPVVHtR2YPi5kE"));
}
