#!/usr/bin/env bash
# Fail if the resolved graph holds more than one copy of an OpenHuman-family
# crate.
#
# Teeny links OpenHuman from the vendor/openhuman submodule, and vendored
# crates name tinytools / tinyinference / tinymemory by git rev or registry
# version. The [patch] tables in the root Cargo.toml redirect all of them onto
# the one vendored path. If a table goes stale after an OpenHuman bump, cargo
# quietly resolves a second copy, and `tinytools::Tool` or `ChatModel` from
# each is a different type: the build may even pass while host tools silently
# fail to register. This is the tripwire.
#
# Reads `cargo metadata` (the whole resolved graph, every target and feature),
# not `cargo tree -d`, which only reports what the current feature set builds.
set -euo pipefail

cd "$(dirname "$0")/.."

WATCHED=(
  openhuman
  openhuman-embed
  tinytools
  tinytools-agent
  tinyinference-core
  tinyinference-llm
  tinyinference-providers
  tinymemory-api
  tinymemory-tools
)

read -r -d '' REPORT <<'PY' || true
import json
import sys

metadata = json.load(sys.stdin)
watched = sys.argv[1:]

copies = {}
for package in metadata["packages"]:
    if package["name"] in watched:
        origin = package["source"] or package["manifest_path"]
        copies.setdefault(package["name"], set()).add(f'{package["version"]} from {origin}')

failed = False
for name in watched:
    found = sorted(copies.get(name, ()))
    if len(found) > 1:
        failed = True
        print(f"DUPLICATE {name}: {len(found)} copies")
        for copy in found:
            print(f"  - {copy}")
    elif found:
        print(f"ok        {name}: {found[0]}")
    else:
        print(f"absent    {name}")

if failed:
    print("\nMore than one copy of an OpenHuman-family crate resolved. Check the")
    print("[patch] tables in the root Cargo.toml against vendor/openhuman/Cargo.toml.")
    sys.exit(1)
PY

cargo metadata --format-version 1 --locked | python3 -c "$REPORT" "${WATCHED[@]}"
