"""A drafting turn's answer as a list of action items, or exit 1 saying why.

    python3 bin/lib/answer.py < answer.txt    # prints the items as JSON

The model answers JSON, sometimes in a code fence or after a stray word.
This takes the first JSON object or array in the text; every item must be an
object with a string `action`. Shared by bin/act and bin/inbound-consume.
"""

from __future__ import annotations

import json
import sys


def items(text: str) -> list[dict]:
    text = text.strip()
    if text.startswith("```"):
        text = text.split("\n", 1)[1] if "\n" in text else ""
        text = text.rsplit("```", 1)[0]
    starts = [i for i in (text.find("{"), text.find("[")) if i >= 0]
    if not starts:
        raise ValueError("no JSON in the answer")
    value, _ = json.JSONDecoder().raw_decode(text[min(starts):])
    found = value if isinstance(value, list) else [value]
    if not all(isinstance(i, dict) and isinstance(i.get("action"), str) for i in found):
        raise ValueError("every item needs an action")
    return found


if __name__ == "__main__":
    try:
        print(json.dumps(items(sys.stdin.read())))
    except ValueError as exc:
        sys.exit(str(exc))
