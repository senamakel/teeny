"""OAuth 1.0a signing, ported from crates/teeny-x/src/oauth_tests.rs."""

from __future__ import annotations

import unittest

from . import support  # noqa: F401  (puts hands/lib on the path)
from teeny_hands.x.oauth import OAuth1, percent_encode


def twitter_example() -> OAuth1:
    """The worked example from X's "Creating a signature" guide."""
    return OAuth1(
        "xvz1evFS4wEEPTGEFPHBog",
        "kAcSOqF21Fu85e7zjz7ZN2U4ZRhfV3WpwPAoE3Z7kBw",
        "370773112-GmHxMAgYyLbNEtIKZeRNFsMKPR9EyMZeS9weJAEb",
        "LswwdoUaIvS8ltyTt5jkRh4J50vUPVVHtR2YPi5kE",
    )


URL = "https://api.twitter.com/1.1/statuses/update.json?include_entities=true"
STATUS = "Hello Ladies + Gentlemen, a signed OAuth request!"
NONCE = "kYjzVBB8Y0ZFabxSWbWovY3uYSQ2pTgmZeNu2VS4cg"
TIMESTAMP = 1_318_622_958


class OAuthTest(unittest.TestCase):
    def test_percent_encodes_everything_but_the_unreserved_set(self):
        self.assertEqual(percent_encode("Ladies + Gentlemen"), "Ladies%20%2B%20Gentlemen")
        self.assertEqual(percent_encode("An encoded string!"), "An%20encoded%20string%21")
        self.assertEqual(percent_encode("Dogs, Cats & Mice"), "Dogs%2C%20Cats%20%26%20Mice")
        self.assertEqual(percent_encode("☃"), "%E2%98%83")
        self.assertEqual(percent_encode("!*'()"), "%21%2A%27%28%29")
        self.assertEqual(percent_encode("az-AZ_09.~"), "az-AZ_09.~")

    def test_builds_the_published_signature_base_string(self):
        base = twitter_example().base_string("post", URL, [("status", STATUS)], NONCE, TIMESTAMP)
        self.assertEqual(
            base,
            "POST&https%3A%2F%2Fapi.twitter.com%2F1.1%2Fstatuses%2Fupdate.json&include_entities%3Dtrue%26"
            "oauth_consumer_key%3Dxvz1evFS4wEEPTGEFPHBog%26oauth_nonce%3DkYjzVBB8Y0ZFabxSWbWovY3uYSQ2pTgmZeNu2VS4cg%26"
            "oauth_signature_method%3DHMAC-SHA1%26oauth_timestamp%3D1318622958%26"
            "oauth_token%3D370773112-GmHxMAgYyLbNEtIKZeRNFsMKPR9EyMZeS9weJAEb%26oauth_version%3D1.0%26"
            "status%3DHello%2520Ladies%2520%252B%2520Gentlemen%252C%2520a%2520signed%2520OAuth%2520request%2521",
        )

    def test_produces_the_published_signature(self):
        header = twitter_example().header_with("POST", URL, [("status", STATUS)], NONCE, TIMESTAMP)
        self.assertIn('oauth_signature="hCtSmYh%2BiHYCEqBWrE7C7hYmtUk%3D"', header)
        self.assertTrue(header.startswith('OAuth oauth_consumer_key="xvz1evFS4wEEPTGEFPHBog", oauth_nonce='), header)
        self.assertIn('oauth_token="370773112-GmHxMAgYyLbNEtIKZeRNFsMKPR9EyMZeS9weJAEb"', header)
        self.assertIn('oauth_version="1.0"', header)

    def test_signs_the_decoded_query_so_an_encoded_value_is_not_encoded_twice(self):
        encoded = twitter_example().base_string(
            "GET", "https://api.x.com/2/tweets/search/recent?query=%40teeny%20-is%3Aretweet", [], NONCE, TIMESTAMP
        )
        self.assertIn("query%3D%2540teeny%2520-is%253Aretweet", encoded)
        self.assertTrue(encoded.startswith("GET&https%3A%2F%2Fapi.x.com%2F2%2Ftweets%2Fsearch%2Frecent&"), encoded)

    def test_fresh_headers_carry_fresh_nonces(self):
        signer = twitter_example()
        self.assertNotEqual(
            signer.header("GET", "https://api.x.com/2/users/me"),
            signer.header("GET", "https://api.x.com/2/users/me"),
        )

    def test_never_prints_its_secrets(self):
        printed = repr(twitter_example()) + str(twitter_example())
        self.assertNotIn("kAcSOqF21Fu85e7zjz7ZN2U4ZRhfV3WpwPAoE3Z7kBw", printed)
        self.assertNotIn("LswwdoUaIvS8ltyTt5jkRh4J50vUPVVHtR2YPi5kE", printed)

    def test_a_consumer_only_signer_sends_no_token(self):
        signer = OAuth1("ck", "cs", "", "")
        url = "https://api.x.com/oauth/request_token?oauth_callback=oob"
        header = signer.header_with("POST", url, [], NONCE, TIMESTAMP)
        self.assertNotIn("oauth_token", header)
        base = signer.base_string("POST", url, [], NONCE, TIMESTAMP)
        self.assertNotIn("oauth_token", base)
        self.assertIn("oauth_callback%3Doob", base)


if __name__ == "__main__":
    unittest.main()
