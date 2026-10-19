from __future__ import annotations
from dataclasses import dataclass, field

DEFAULT_LINK_ALLOWLIST = ()


@dataclass(frozen=True)
class AllowlistEntry:
    host: str
    path_prefix: str | None = None
    root_only: bool = False


@dataclass
class LinkFilterResult:
    text: str
    removed: list = field(default_factory=list)
    kept: list = field(default_factory=list)


def parse_allowlist(entries):
    return []


def split_setting(value):
    return []


def configured_allowlist():
    return []


def find_links(text):
    return []


def judge_link(link, allowlist):
    return "no"


def filter_links(text, allowlist):
    return LinkFilterResult(text)
