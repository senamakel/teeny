"""The read-through cache in state/hands.db, ported from crates/teeny-x/src/
cache_tests.rs (itself legacy/test/x-cache.test.ts). X bills every post a read
returns: nothing is bought twice, a list asked for twice in one window is one
request, and the store failing costs a hit, never a read."""

from __future__ import annotations

import threading
from datetime import datetime, timezone

from .support import Scratch, me
from teeny_hands import hand
from teeny_hands.x import cache
from teeny_hands.x.client import XError
from teeny_hands.x.model import SearchOptions


def post(post_id: str) -> dict:
    return {"id": post_id, "text": f"post {post_id}", "author_id": "u1", "conversation_id": post_id}


def iso(ms: int) -> str:
    return datetime.fromtimestamp(ms / 1000, tz=timezone.utc).isoformat().replace("+00:00", "Z")


class CacheTest(Scratch):
    def setUp(self):
        super().setUp()
        self.at(1_000_000)
        self.get7 = self.x.route("GET", "/2/tweets/7", {"data": post("7")})
        self.missing = self.x.route("GET", "/2/tweets/404", {"errors": [{"title": "Not Found Error"}]})
        self.search = self.x.route("GET", "/2/tweets/search/recent", {"data": [post("1"), post("2")]})

    def calls(self, route) -> int:
        return route.times[0]

    def test_serves_a_post_from_the_store_after_the_first_read(self):
        reads = cache.reads()
        first = reads.get_post("7")
        self.assertEqual(cache.reads().get_post("7"), first)
        self.assertEqual(first["text"], "post 7")
        self.assertEqual(self.calls(self.get7), 1)

    def test_remembers_every_post_a_search_or_a_conversation_brought_back(self):
        reads = cache.reads()
        reads.search_posts("$TINY", SearchOptions())
        reads.get_conversation("9", 25)
        self.assertEqual(reads.get_post("1")["text"], "post 1")
        self.assertEqual(len(self.x.requests("GET", "/2/tweets/1")), 0)

    def test_keeps_a_search_only_for_its_window_keyed_on_query_and_options(self):
        self.configure(X_CACHE_SEARCH_TTL_MINUTES="1")
        reads = cache.reads()
        reads.search_posts("$TINY", SearchOptions(max_results=10))
        reads.search_posts("$TINY", SearchOptions(max_results=10))
        self.assertEqual(self.calls(self.search), 1)
        reads.search_posts("$TINY", SearchOptions(max_results=20))
        self.assertEqual(self.calls(self.search), 2)
        self.at(1_000_000 + 60.001)
        reads.search_posts("$TINY", SearchOptions(max_results=10))
        self.assertEqual(self.calls(self.search), 3)

    def test_rounds_a_moving_time_window_to_the_search_ttl(self):
        start = 10 * 60_000
        self.at(start / 1000)

        def window(ms):
            return SearchOptions(start_time=iso(ms - 3_600_000), end_time=iso(ms - 60_000))

        cache.reads().search_posts("$TINY", window(start))
        self.at((start + 30_000) / 1000)
        cache.reads().search_posts("$TINY", window(start + 30_000))
        self.assertEqual(self.calls(self.search), 1)

    def test_remembers_that_a_post_was_missing_but_only_briefly(self):
        self.at(0)
        self.assertIsNone(cache.reads().get_post("404"))
        self.assertIsNone(cache.reads().get_post("404"))
        self.assertEqual(self.calls(self.missing), 1)
        self.at(3600 + 0.001)
        cache.reads().get_post("404")
        self.assertEqual(self.calls(self.missing), 2)

    def test_keeps_a_post_for_the_configured_days(self):
        self.configure(X_CACHE_POST_TTL_DAYS="2")
        cache.reads().get_post("7")
        self.at(1_000_000 + 2 * 86400 - 1)
        cache.reads().get_post("7")
        self.assertEqual(self.calls(self.get7), 1)
        self.at(1_000_000 + 2 * 86400 + 1)
        cache.reads().get_post("7")
        self.assertEqual(self.calls(self.get7), 2)

    def test_shares_one_flight_between_callers_asking_the_same_thing_at_once(self):
        slow = self.x.route("GET", "/2/tweets/3", {"data": post("3")}, delay=0.3)
        results = []

        def ask():
            results.append(cache.reads().get_post("3"))

        threads = [threading.Thread(target=ask) for _ in range(3)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()
        self.assertEqual(len(results), 3)
        self.assertTrue(all(r == results[0] for r in results))
        self.assertEqual(self.calls(slow), 1)

    def test_goes_to_x_when_the_store_is_broken(self):
        (self.root / "state").mkdir(exist_ok=True)
        (self.root / "state" / "hands.db").write_bytes(b"this is not a database" * 100)
        self.assertEqual(cache.reads().get_post("7")["id"], "7")
        self.assertEqual(cache.reads().get_post("7")["id"], "7")
        self.assertEqual(self.calls(self.get7), 2)

    def test_a_failed_read_is_not_cached(self):
        failing = self.x.route("GET", "/2/tweets/search/recent", {"title": "Service Unavailable"}, status=503)
        with self.assertRaisesRegex(XError, "503"):
            cache.reads().search_posts("q", SearchOptions())
        with self.assertRaisesRegex(XError, "503"):
            cache.reads().search_posts("q", SearchOptions())
        self.assertEqual(self.calls(failing), 2)

    def test_x_cache_enabled_decides_whether_reads_go_through_the_store(self):
        self.configure(X_CACHE_ENABLED="false")
        cache.reads().get_post("7")
        cache.reads().get_post("7")
        self.assertEqual(self.calls(self.get7), 2)
        self.configure(X_CACHE_ENABLED="true")
        cache.reads().get_post("7")
        cache.reads().get_post("7")
        self.assertEqual(self.calls(self.get7), 3)

    def test_looks_an_account_up_on_x_every_time(self):
        users = self.x.route("GET", "/2/users/by/username/stonks", {"data": {"id": "9", "username": "stonks"}})
        self.assertEqual(cache.reads().user_by_username("stonks"), {"id": "9", "username": "stonks"})
        cache.reads().user_by_username("stonks")
        self.assertEqual(self.calls(users), 2)

    def test_keeps_the_account_s_identity_per_access_token(self):
        whoami = me(self.x)
        self.assertEqual(cache.reads().identity(), {"id": "1000", "username": "teenydroid"})
        cache.reads().identity()
        self.assertEqual(self.calls(whoami), 1)
        # Re-authorized as another account: the old answer is not this token's.
        self.configure(X_ACCESS_TOKEN="at-ANOTHER")
        cache.reads().identity()
        self.assertEqual(self.calls(whoami), 2)
        rows = [r["key"] for r in hand.db().execute("SELECT key FROM x_cache WHERE kind = 'identity'")]
        self.assertTrue(rows)
        self.assertTrue(all("FAKE" not in key and "ANOTHER" not in key for key in rows), rows)


class KeysTest(Scratch):
    def test_keys_match_the_legacy_cache(self):
        # sha256 of the JSON the TypeScript keyed on, first 24 hex characters.
        self.assertEqual(
            cache.search_key("$TINY", SearchOptions(max_results=10), 600),
            "search:" + cache.digest('{"query":"$TINY","maxResults":10}'),
        )
        self.assertEqual(cache.digest("x"), "2d711642b726b04401627ca9")

    def test_ttls_follow_the_configuration(self):
        self.configure(X_CACHE_POST_TTL_DAYS="30", X_CACHE_SEARCH_TTL_MINUTES="5")
        ttl = cache.Ttl.from_settings()
        self.assertEqual(ttl.post, 30 * 24 * 3600)
        self.assertEqual(ttl.search, 300)
        self.assertEqual(ttl.conversation, 300)
        self.assertEqual(ttl.missing, 3600)
