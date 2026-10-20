"""hands/guard's verbs (docs/hands.md, "guard").

  check     stdin {"text", "surface"?, "kind"?, "replying_to"?, "max_emoji"?, "context"?}
            -> {"ok": true, "text"}; held -> refused "held" with the category
            and hold id; refused (rewrite it) -> refused <why>, rewrite true
  inbound   stdin {"text", "surface"?, "context"?} -> {"ok": true}, or held
  style     stdin {"text", "replying_to"?, "max_emoji"?} -> {"text", "length", "emoji"}
  holds list [--status S] [--limit N] | approve ID [--note T] | reject ID [--note T]
  links list | approve HOST[/PATH]

Releasing an approved draft (publishing it) belongs to hands/x, which holds
the X keys: approving one here records the decision, and hands/x releases
through `holds.claim_release` / `unclaim_release`.
"""

from __future__ import annotations

import argparse
from typing import Any, Mapping

from .. import hand
from . import gate, holds
from .links import parse_allowlist
from .style import apply_house_style, count_emoji, post_length


def _body(body: Any, *keys: str) -> dict[str, Any]:
    """The JSON body, with a string `text`, and nothing but `keys` besides."""
    if not isinstance(body, dict):
        raise hand.HandError("stdin must be a JSON object with a text")
    if not isinstance(body.get("text"), str):
        raise hand.HandError("text must be a string")
    unknown = set(body) - {"text", *keys}
    if unknown:
        raise hand.HandError(f"unknown field(s): {', '.join(sorted(unknown))}")
    return body


def _optional(body: dict[str, Any], key: str, kind: type, what: str) -> Any:
    value = body.get(key)
    if value is not None and (not isinstance(value, kind) or isinstance(value, bool)):
        raise hand.HandError(f"{key} must be {what}")
    return value


def _max_emoji(body: dict[str, Any]) -> int | None:
    value = _optional(body, "max_emoji", int, "a whole number")
    if value is not None and value < 0:
        raise hand.HandError("max_emoji must not be negative")
    return value


def check(args: argparse.Namespace, body: Any) -> Mapping[str, Any]:
    body = _body(body, "surface", "kind", "replying_to", "max_emoji", "context")
    verdict = gate.check_outbound(
        body["text"],
        surface=_optional(body, "surface", str, "a string") or "x",
        kind=_optional(body, "kind", str, "a string") or "post",
        replying_to=_optional(body, "replying_to", str, "a string"),
        max_emoji=_max_emoji(body),
        context=_optional(body, "context", dict, "an object"),
    )
    if verdict.kind == "hold":
        raise hand.Refused("held", category=verdict.category, hold_id=verdict.hold_id)
    if verdict.kind == "refuse":
        raise hand.Refused(verdict.reason, rewrite=True)
    return {"text": verdict.text}


def inbound(args: argparse.Namespace, body: Any) -> Mapping[str, Any]:
    body = _body(body, "surface", "context")
    verdict = gate.inbound(
        body["text"],
        surface=_optional(body, "surface", str, "a string") or "x",
        context=_optional(body, "context", dict, "an object"),
    )
    if verdict.kind == "hold":
        raise hand.Refused("held", category=verdict.category, hold_id=verdict.hold_id)
    return {}


def style(args: argparse.Namespace, body: Any) -> Mapping[str, Any]:
    body = _body(body, "replying_to", "max_emoji")
    max_emoji = _max_emoji(body)
    styled = apply_house_style(
        body["text"],
        max_emoji=gate.max_emoji_per_post() if max_emoji is None else max_emoji,
        replying_to=_optional(body, "replying_to", str, "a string"),
    )
    return {"text": styled.text, "length": post_length(styled.text), "emoji": count_emoji(styled.text)}


def holds_list(args: argparse.Namespace, body: Any) -> Mapping[str, Any]:
    return {"holds": holds.list_holds(args.status, args.limit), "counts": holds.counts()}


def _review(decision: str):
    def review(args: argparse.Namespace, body: Any) -> Mapping[str, Any]:
        reviewed = holds.review_hold(args.id, decision, args.note, refresh_links=gate.refresh_approved_links)
        if reviewed is None:
            raise hand.HandError(f"no hold {args.id}")
        return {"hold": reviewed}

    return review


def links_list(args: argparse.Namespace, body: Any) -> Mapping[str, Any]:
    return {"links": holds.approved_link_rows()}


def links_approve(args: argparse.Namespace, body: Any) -> Mapping[str, Any]:
    entry = args.entry.strip().lower()
    parsed = parse_allowlist([entry])
    if not parsed or not parsed[0].host:
        raise hand.HandError("give a host, or host/path, to allow")
    holds.allow_link(entry, None)
    gate.refresh_approved_links()
    return {"entry": entry}


def parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(prog="hands/guard", description="Teeny's content filter.")
    verbs = p.add_subparsers(dest="verb")

    for name, handler in (("check", check), ("inbound", inbound), ("style", style)):
        verbs.add_parser(name).set_defaults(handler=handler, takes_body=True)

    holds_p = verbs.add_parser("holds").add_subparsers(dest="action")
    listing = holds_p.add_parser("list")
    listing.add_argument("--status", choices=holds.STATUSES)
    listing.add_argument("--limit", type=int)
    listing.set_defaults(handler=holds_list)
    for decision, name in (("approved", "approve"), ("rejected", "reject")):
        review = holds_p.add_parser(name)
        review.add_argument("id", type=int)
        review.add_argument("--note")
        review.set_defaults(handler=_review(decision))

    links_p = verbs.add_parser("links").add_subparsers(dest="action")
    links_p.add_parser("list").set_defaults(handler=links_list)
    approve = links_p.add_parser("approve")
    approve.add_argument("entry", metavar="HOST[/PATH]")
    approve.set_defaults(handler=links_approve)
    return p


def main(argv: list[str]) -> int:
    return hand.run("guard", parser(), argv)
