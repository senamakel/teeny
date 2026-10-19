"""OAuth 1.0a request signing (RFC 5849, HMAC-SHA1), as X signs user-context
calls: every write, `/2/users/me`, and the reads that only the account itself
may make.

The signature covers the method, the URL without its query, the query
parameters, any form-encoded body parameters and the `oauth_*` set. A JSON
body is not a parameter and is not signed, which is what X expects.

Ported from crates/teeny-x/src/oauth.rs.
"""

from __future__ import annotations

import base64
import hashlib
import hmac
import itertools
import os
import time
import urllib.parse
from typing import Iterable, Sequence

# RFC 5849 section 3.6: everything but ALPHA, DIGIT, `-`, `.`, `_` and `~`.
_UNRESERVED = "-._~"


def percent_encode(value: str) -> str:
    """Percent-encodes a value the way RFC 5849 requires: UTF-8, upper-case
    hex, and only the unreserved set left alone. (Not `encodeURIComponent`,
    which leaves `!*'()` bare.)"""
    return urllib.parse.quote(value, safe=_UNRESERVED)


class OAuth1:
    """The account's user-context credentials, able to sign a request. Its
    repr names no secret, so a traceback or a log line cannot leak one."""

    __slots__ = ("_consumer_key", "_consumer_secret", "_token", "_token_secret")

    def __init__(self, consumer_key: str, consumer_secret: str, token: str, token_secret: str) -> None:
        self._consumer_key = consumer_key
        self._consumer_secret = consumer_secret
        self._token = token
        self._token_secret = token_secret

    def __repr__(self) -> str:
        return "OAuth1(<redacted>)"

    __str__ = __repr__

    def header(self, method: str, url: str, form: Sequence[tuple[str, str]] = ()) -> str:
        """The `Authorization` header for one request, with a fresh nonce and
        the current time. `url` carries the query; `form` is a form-encoded
        body, empty for a JSON body or none."""
        return self.header_with(method, url, form, _nonce(), int(time.time()))

    def header_with(
        self, method: str, url: str, form: Sequence[tuple[str, str]], nonce: str, timestamp: int
    ) -> str:
        """`header` with the nonce and timestamp given, so a signature can be
        checked against a known one."""
        base = self.base_string(method, url, form, nonce, timestamp)
        key = f"{percent_encode(self._consumer_secret)}&{percent_encode(self._token_secret)}"
        digest = hmac.new(key.encode(), base.encode(), hashlib.sha1).digest()
        signature = base64.b64encode(digest).decode()
        oauth = sorted([*self._oauth_params(nonce, timestamp), ("oauth_signature", signature)])
        fields = ", ".join(f'{percent_encode(name)}="{percent_encode(value)}"' for name, value in oauth)
        return f"OAuth {fields}"

    def base_string(
        self, method: str, url: str, form: Sequence[tuple[str, str]], nonce: str, timestamp: int
    ) -> str:
        """The signature base string (RFC 5849 section 3.4.1). The query is
        decoded before it is re-encoded, so a value that arrived encoded is
        not encoded twice."""
        parts = urllib.parse.urlsplit(url)
        query = urllib.parse.parse_qsl(parts.query, keep_blank_values=True)
        bare = urllib.parse.urlunsplit((parts.scheme, parts.netloc, parts.path, "", ""))
        params = sorted(
            (percent_encode(name), percent_encode(value))
            for name, value in itertools.chain(query, form, self._oauth_params(nonce, timestamp))
        )
        normalized = "&".join(f"{name}={value}" for name, value in params)
        return f"{method.upper()}&{percent_encode(bare)}&{percent_encode(normalized)}"

    def _oauth_params(self, nonce: str, timestamp: int) -> Iterable[tuple[str, str]]:
        """The `oauth_*` set. A signer with no token (the app alone, asking
        for a request token) sends no `oauth_token` at all, as RFC 5849 has it."""
        params = [
            ("oauth_consumer_key", self._consumer_key),
            ("oauth_nonce", nonce),
            ("oauth_signature_method", "HMAC-SHA1"),
            ("oauth_timestamp", str(timestamp)),
        ]
        if self._token:
            params.append(("oauth_token", self._token))
        params.append(("oauth_version", "1.0"))
        return params


def _nonce() -> str:
    """A nonce only has to be unique per timestamp, not secret, but the OS
    hands out random bytes for free."""
    return os.urandom(16).hex()
