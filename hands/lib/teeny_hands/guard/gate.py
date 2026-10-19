from __future__ import annotations
from dataclasses import dataclass, field
from .. import hand

APPROVED_LINKS_TTL = 60.0


@dataclass(frozen=True)
class Verdict:
    kind: str
    text: str
    reason: str
    category: str | None = None
    hold_id: int | None = None


@dataclass
class OutboundResult:
    held: object
    text: str
    fixes: list = field(default_factory=list)
    hold_id: int | None = None


def screen(text, direction):
    return True


def refresh_approved_links():
    pass


def gate_inbound(text, context=None):
    return None


def gate_outbound(text, context=None):
    return OutboundResult(None, text)


def check_outbound(text, *, surface="x", kind="post", replying_to=None, max_emoji=None, context=None):
    return Verdict("allow", text, "")


def inbound(text, *, surface="x", context=None):
    return Verdict("allow", text, "")
