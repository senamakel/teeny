"""The one way to publish. Every write runs, in this order and in this one
invocation: the guard (`check_outbound`: filter, house style, the emoji cap),
the 280 cap on what the guard returned, the posting rules, `DRY_RUN`, X, and
the record. A job script that forgets a step still cannot post unchecked,
because there is no step to forget. Ported from
crates/teeny-x/src/publisher.rs and the host side of
crates/teeny-agent/src/tools/post.rs and teeny-jobs/src/reply/job.rs.

What goes out is always the guard's text, never the caller's.
"""

from __future__ import annotations

import sys
from typing import Any, Mapping

from .. import hand
from . import posting_rules as rules
from .client import XApi, number_setting

try:
    # Imported by name, so a test can stand in for the other hand's gate.
    from ..guard.gate import check_outbound
except ImportError:  # the guard hand is not installed: nothing is published

    def guard_unavailable(text: str, **_: Any) -> Any:
        raise hand.HandError("the content guard (hands/guard) is not installed, so nothing can be published")

    check_outbound = guard_unavailable
else:

    def guard_unavailable(text: str, **_: Any) -> Any:
        raise hand.HandError("the content guard (hands/guard) is not installed, so nothing can be published")


Json = dict[str, Any]


def _max_emoji() -> int:
    return int(number_setting("MAX_EMOJI_PER_POST", 1, 0, 10))


def _guarded(text: str, kind: str, replying_to: str | None = None, context: Mapping[str, Any] | None = None) -> str:
    """The guard's say on `text`; the text to publish when it allows."""
    verdict = check_outbound(
        text,
        surface="x",
        kind=kind,
        replying_to=replying_to,
        max_emoji=_max_emoji(),
        context=dict(context or {}),
    )
    if verdict.kind == "allow" and isinstance(verdict.text, str) and verdict.text.strip():
        return verdict.text
    if verdict.kind == "hold":
        raise hand.Refused("held", category=verdict.category, hold_id=verdict.hold_id)
    if verdict.kind == "refuse":
        raise hand.Refused(verdict.reason, rewrite=True)
    # An answer this hand does not understand is not an "allow".
    raise hand.HandError(f"the guard answered {verdict.kind!r}, which hands/x does not publish on")


def _draft(body: Any, *, quote: bool) -> tuple[str, list[str], str | None, str | None]:
    """The stdin body: text, media ids, a Giphy id, and for an own post the
    post it quotes. Anything else in it is refused, so a URL cannot ride in
    under a field this hand ignores."""
    if not isinstance(body, Mapping):
        raise hand.HandError('stdin must be a JSON object with "text"')
    allowed = {"text", "media_ids", "gif_id"} | ({"quote_of"} if quote else set())
    unknown = sorted(set(body) - allowed)
    if unknown:
        raise hand.HandError(f"unknown field(s) in the body: {', '.join(unknown)}")
    text = body.get("text")
    if not isinstance(text, str) or not text.strip():
        raise hand.HandError('"text" must be a non-empty string')
    media_ids = body.get("media_ids") or []
    if not isinstance(media_ids, list) or not all(isinstance(m, str) and m for m in media_ids):
        raise hand.HandError('"media_ids" must be a list of media ids')
    gif_id = body.get("gif_id")
    if gif_id is not None and not isinstance(gif_id, str):
        raise hand.HandError('"gif_id" must be a string')
    quote_of = body.get("quote_of")
    if quote_of is not None and not rules.is_x_id(quote_of):
        raise hand.HandError('"quote_of" must be a numeric X post id')
    return text, list(media_ids), gif_id, quote_of


def _dry(text: str, kind: str, **extra: Any) -> Json:
    print(f"hands/x: dry run: would have published a {kind}: {text!r}", file=sys.stderr)
    return {"ok": True, "dry_run": True, "would_post": text, "kind": kind, **extra}


def _db():
    conn = hand.db()
    rules.ensure_schema(conn)
    return conn


def post(body: Any, x: XApi | None = None) -> Json:
    """An own post, or a quote: guard, 280, attachments, then the gap under
    the process-wide posting lock, so two jobs firing in the same minute
    cannot both read the ledger before either is recorded."""
    text, media_ids, gif_id, quote_of = _draft(body, quote=True)
    kind = "quote" if quote_of else "post"
    text = _guarded(text, kind)
    rules.check_length(text)
    live = not hand.dry_run()
    conn = _db()
    rules.check_attachments(conn, media_ids, gif_id, live=live)
    with hand.lock("posting"):
        rules.check_gap(conn)
        if not live:
            return _dry(text, kind)
        x = x or XApi.from_settings()
        post_id = x.create_post(text, quote=quote_of, media_ids=media_ids)
        # The post is out. A ledger that cannot record it is logged, not
        # raised: an error here would invite a retry that posts it twice.
        try:
            rules.record_post(conn, post_id, kind, text, quote_of=quote_of, media=media_ids)
        except Exception as exc:  # noqa: BLE001
            print(f"hands/x: published {post_id} but could not record it: {type(exc).__name__}", file=sys.stderr)
    return {"id": post_id, "kind": kind, "text": text}
