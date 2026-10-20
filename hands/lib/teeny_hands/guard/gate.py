"""The gate every message passes through twice: once on the way in, once on
the way out.

Inbound, a prohibited message is not answered: not declined, not
deflected, not processed. The drafter never sees it, nothing about it is
remembered, and the journal records that something was held and why,
without the text.

Outbound, the account's own draft is held to a stricter reading
(profanity counts, and every link is checked against the allowlist), and
the caller gets back either clean text to publish or a reason not to. A
clean question can still get a dirty answer, from a model with a bad day
or a web page with a plan.

Checks run cheapest first and each is sufficient to stop the turn: links
are cut, the lexicon is read, and only then is the judge asked.
A port of `crates/teeny-safety/src/gate.rs` (itself
`legacy/src/safety/gate.ts`), with `check_outbound`, the seam the X
Publisher guard called (the legacy `x_post` tool's checks).

`gate_inbound` and `gate_outbound` are the Rust's `SafetyGate::inbound` and
`outbound`; `inbound` and `check_outbound` are what hands/x calls.
"""

from __future__ import annotations

import dataclasses
import logging
import time
from dataclasses import dataclass, field
from typing import Any

from .. import hand
from . import holds
from .judge import judge
from .links import AllowlistEntry, configured_allowlist, filter_links, parse_allowlist
from .screen import ARTICLE, INBOUND, NARRATIVE, OUTBOUND, screen_text
from .style import MAX_POST_LENGTH, apply_house_style, count_emoji, post_length

log = logging.getLogger(__name__)

# How long approved links are cached before the log is asked again.
APPROVED_LINKS_TTL = 60.0

# `MAX_EMOJI_PER_POST`'s default and bounds, as the Rust config read it.
DEFAULT_MAX_EMOJI = 1
MAX_EMOJI_LIMIT = 10


@dataclass(frozen=True)
class Held:
    """Why a text was held."""

    # A lexicon category, the judge's category, `links`, or `unknown`.
    category: str
    # For the log and the journal. Never contains the text.
    reason: str
    # Which check held it: screen, judge or links.
    by: str
    # The guard_holds row, when one was written.
    hold_id: int | None = None


@dataclass
class OutboundResult:
    """An outbound check: the decision, and the text with disallowed links cut."""

    # None when clean.
    held: Held | None
    # What to publish. Only meaningful when clean.
    text: str
    # What was changed, for logging. Empty when the draft was already clean.
    fixes: list[str] = field(default_factory=list)
    # The hold written for this check, if any: the held draft, or the cut
    # links of a draft that was otherwise clean.
    hold_id: int | None = None


@dataclass(frozen=True)
class Verdict:
    """What may happen to a text."""

    # "allow": publish `text`, the house style applied and the gate passed.
    # "hold": the gate held it and recorded it for review. Do not rephrase it.
    # "refuse": it cannot go out as written (too long, too many emoji, a link
    # off the allowlist); the model may rewrite it and try again.
    kind: str
    text: str
    # Why, for hold and refuse; "" for allow.
    reason: str
    category: str | None = None
    hold_id: int | None = None


# --- settings --------------------------------------------------------------------


def judge_enabled() -> bool:
    """`SAFETY_JUDGE`, default on. Off without the judge is the Rust's
    `SafetyGateOptions.judge = None`: the lexicon and the allowlist alone.
    Only a clear off turns it off; a value it cannot read leaves the judge
    on, because a typo must not switch half the filter off."""
    value = hand.setting("SAFETY_JUDGE")
    return value is None or value.strip().lower() not in ("0", "false", "no", "off")


def max_emoji_per_post() -> int:
    """`MAX_EMOJI_PER_POST`, a whole number 0..=10, default 1. The Rust refused
    to start on a bad value; a hand has no start, so a bad value is the default."""
    raw = hand.setting("MAX_EMOJI_PER_POST")
    if raw is None:
        return DEFAULT_MAX_EMOJI
    try:
        value = float(raw.strip())
    except ValueError:
        return DEFAULT_MAX_EMOJI
    if not value.is_integer() or not 0 <= value <= MAX_EMOJI_LIMIT:
        return DEFAULT_MAX_EMOJI
    return int(value)


# --- the allowlist ---------------------------------------------------------------

# Operator approvals, per checkout: TEENY_ROOT -> (entries, when read).
_approved: dict[str, tuple[list[AllowlistEntry], float | None]] = {}


def allowlist() -> list[AllowlistEntry]:
    """The built-in list and `LINK_ALLOWLIST`, plus whatever an operator has
    approved since, read on a short cache so an approval taken in one process
    reaches a long-running one without a restart. A log that cannot be read
    leaves the approved set as it was, never empties it."""
    key = str(hand.root())
    entries, at = _approved.get(key, ([], None))
    if at is None or time.monotonic() - at > APPROVED_LINKS_TTL:
        try:
            entries = parse_allowlist(holds.allowed_links())
        except Exception as error:  # noqa: BLE001 - keep the last set, whatever broke
            log.warning("safety: could not read approved links; keeping the last set: %s", error)
        _approved[key] = (entries, time.monotonic())
    return [*configured_allowlist(), *entries]


def refresh_approved_links() -> None:
    """Forgets the cached approvals, so the next check reads the log."""
    for key, (entries, _) in list(_approved.items()):
        _approved[key] = (entries, None)


# --- the checks ------------------------------------------------------------------


def screen(text: str, direction: str) -> bool:
    """The lexicon alone, synchronous, for scrubbing context lines cheaply.
    True when the text is clean."""
    return screen_text(text, direction) is None


def _record(**entry: Any) -> int | None:
    """Writes a hold for review. Never fails: the decision is already made."""
    try:
        return holds.record(**entry)["id"]
    except Exception as error:  # noqa: BLE001 - a broken log must not undo a hold
        log.warning("safety: could not record a hold: %s", error)
        return None


def _ask(text: str, direction: str) -> Held | None:
    if not judge_enabled() or not text.strip():
        return None
    verdict = judge(text, direction)
    if verdict.verdict == "clean":
        return None
    category = verdict.category or "unknown"
    level = logging.INFO if direction == INBOUND else logging.WARNING
    log.log(level, "safety: held by the judge (%s, %s): %s", direction, category, verdict.reason)
    return Held(category, verdict.reason or "the judge held it", "judge")


def gate_inbound(text: str, context: dict[str, Any] | None = None) -> Held | None:
    """Whether a message from somebody else is something the account
    processes at all: None when it is. With a context (`surface` and where it
    came from), a hold is recorded for review."""
    found = screen_text(text, INBOUND)
    if found is not None:
        log.info("safety: inbound held by the screen (%s, %s)", found.category, found.term)
        held: Held | None = Held(found.category, f'contains "{found.term}"', "screen")
    else:
        held = _ask(text, INBOUND)
    if held is not None and context is not None:
        kind = context.get("kind") or ("task" if context.get("surface") == "task" else "message")
        hold_id = _record(
            direction=INBOUND,
            kind=kind,
            context=context,
            category=held.category,
            reason=held.reason,
            by=held.by,
            text=text,
        )
        held = dataclasses.replace(held, hold_id=hold_id)
    return held


def gate_outbound(text: str, context: dict[str, Any] | None = None) -> OutboundResult:
    """The account's own words, checked before they leave. The text handed
    back has its links filtered; the caller publishes that. A cut link is
    a hold of its own even when the rest goes out, because it is the
    decision a reviewer most often wants to overturn."""
    fixes: list[str] = []
    links = filter_links(text, allowlist())
    cut = bool(links.removed)
    if cut:
        fixes.append(f"cut {len(links.removed)} link(s) not on the allowlist")
        log.warning("safety: cut links from an outbound draft: %s", links.removed)

    # The serial's posts and the articles written from its lore are read
    # under the narrative profile; every other draft under the strict one.
    surface = context.get("surface") if context else None
    direction = {"narrative": NARRATIVE, "article": ARTICLE}.get(surface or "", OUTBOUND)
    held = _outbound_decision(links.text, cut, direction)
    hold_id = None
    if context is not None:
        removed = links.removed if cut else None
        if held is not None:
            hold_id = _record(
                direction=OUTBOUND,
                kind=context.get("kind") or "draft",
                context=context,
                category=held.category,
                reason=held.reason,
                by=held.by,
                text=text,
                links=removed,
            )
            held = dataclasses.replace(held, hold_id=hold_id)
        elif cut:
            hold_id = _record(
                direction=OUTBOUND,
                kind="link",
                context=context,
                category="links",
                reason="; ".join(r["reason"] for r in links.removed),
                by="links",
                text=text,
                links=removed,
            )
    return OutboundResult(held, links.text, fixes, hold_id)


def _outbound_decision(cleaned: str, links_cut: bool, direction: str) -> Held | None:
    found = screen_text(cleaned, direction)
    if found is not None:
        log.warning("safety: outbound held by the screen (%s, %s)", found.category, found.term)
        return Held(found.category, f'contains "{found.term}"', "screen")
    # Nothing left to say once the links are gone is not a message to send.
    if links_cut and not any(c.isascii() and c.isalnum() for c in cleaned):
        return Held("links", "the draft was only a disallowed link", "links")
    return _ask(cleaned, direction)


# --- what hands/x calls ------------------------------------------------------------


def _context(surface: str, context: dict[str, Any] | None) -> dict[str, Any]:
    """The hold context: the caller's details, with `surface` (the argument
    wins over any in the dict) checked against the hold log's surfaces."""
    if surface not in holds.SURFACES:
        raise hand.HandError(f"surface must be one of {', '.join(holds.SURFACES)}, not {surface!r}")
    return {**(context or {}), "surface": surface}


def inbound(text: str, *, surface: str = "x", context: dict[str, Any] | None = None) -> Verdict:
    """Screens third-party text before a model sees it: "allow" or "hold",
    and a hold is recorded for review."""
    held = gate_inbound(text, _context(surface, context))
    if held is None:
        return Verdict("allow", text, "")
    return Verdict("hold", "", held.reason, held.category, held.hold_id)


def check_outbound(
    text: str,
    *,
    surface: str = "x",
    kind: str = "post",
    replying_to: str | None = None,
    max_emoji: int | None = None,
    context: dict[str, Any] | None = None,
) -> Verdict:
    """The checks a posting tool runs before anything goes out, in the legacy
    `x_post` order: the hard limits are refused so the model learns from
    them, the rest of the house style is applied silently, and the result
    goes through `gate_outbound` with the draft's context.

    `kind` is what is being published ("post", "reply", ...); a hold of it
    is a `draft` hold with the kind kept in its context, unless it names a
    hold kind itself (`prompt` for a picture). `max_emoji` None is
    `MAX_EMOJI_PER_POST`."""
    if max_emoji is None:
        max_emoji = max_emoji_per_post()
    hold_context = _context(surface, context)
    if kind in holds.KINDS:
        hold_context["kind"] = kind
    else:
        hold_context["post_kind"] = kind
    if replying_to:
        hold_context["replying_to"] = replying_to

    length = post_length(text)
    if length > MAX_POST_LENGTH:
        return Verdict("refuse", "", f"the text is {length} characters, over the {MAX_POST_LENGTH} limit.")
    emoji = count_emoji(text)
    if emoji > max_emoji:
        return Verdict(
            "refuse",
            "",
            f"the text has {emoji} emoji and this account allows at most {max_emoji} per post."
            " Rewrite it and call the tool again.",
        )
    styled = apply_house_style(text, max_emoji=max_emoji, max_length=MAX_POST_LENGTH, replying_to=replying_to)
    result = gate_outbound(styled.text, hold_context)
    if result.held is not None:
        return Verdict(
            "hold",
            "",
            f"this account does not publish that ({result.held.category})."
            " Do not rephrase it; say something else or say nothing.",
            result.held.category,
            result.held.hold_id,
        )
    if result.fixes:
        return Verdict(
            "refuse",
            "",
            "the text links to a site that is not on the allowlist. This account only links to its own sites,"
            " code and community. Drop the link and call the tool again.",
        )
    return Verdict("allow", result.text, "")
