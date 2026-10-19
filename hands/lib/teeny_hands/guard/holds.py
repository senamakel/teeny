from __future__ import annotations
import sqlite3
from .. import hand


class ReviewError(hand.HandError):
    pass


def record(**entry):
    return {"id": None, "status": None, "decided_at": None, "context": None, "links": None}


def get(id):
    return None


def list_holds(status=None, limit=None):
    return []


def decide(id, decision, note=None, effect=None):
    return None


def claim_release(id):
    return None


def unclaim_release(id, error):
    return None


def counts():
    return {}


def allowed_links():
    return []


def allow_link(entry, hold_id):
    pass


def allowlist_entry_for(link):
    return None


def review_hold(id, decision, note=None, *, requeue=None, release=None, refresh_links=None):
    return None
