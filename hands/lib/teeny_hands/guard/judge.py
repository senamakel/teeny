from __future__ import annotations
from dataclasses import dataclass

MAX_JUDGED_LENGTH = 6000
RULES = ""


@dataclass(frozen=True)
class JudgeVerdict:
    verdict: str
    category: str | None
    reason: str


def judge(text, direction):
    return JudgeVerdict("clean", None, "")
