from __future__ import annotations
from dataclasses import dataclass, field

MAX_POST_LENGTH = 280


@dataclass
class HouseStyleResult:
    text: str
    fixes: list = field(default_factory=list)


def count_emoji(text):
    return -1


def post_length(text):
    return len(text)


def truncate_at_boundary(text, max_length):
    return text


def apply_house_style(draft, *, max_emoji=1, max_length=MAX_POST_LENGTH, replying_to=None):
    return HouseStyleResult(draft)


def asks_the_reader(text):
    return None


def without_the_ask(text):
    return text
