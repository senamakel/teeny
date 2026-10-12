#!/usr/bin/env bash
# Tests for docker/entrypoint.sh, run as an ordinary user against a scratch
# data root: the layout, the legacy migration, the Chrome lock sweep and the
# exec. The root-only half (ownership heal, gosu) needs a container; this is
# everything else.
#
#   docker/entrypoint_test.sh
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
entrypoint="$here/entrypoint.sh"
scratch="$(mktemp -d "${TMPDIR:-/tmp}/teeny-entrypoint.XXXXXX")"
trap 'rm -rf "$scratch"' EXIT

failures=0
pass() { printf 'ok   %s\n' "$1"; }
fail() { printf 'FAIL %s\n' "$1" >&2; failures=$((failures + 1)); }
check() { local name="$1"; shift; if "$@"; then pass "$name"; else fail "$name"; fi; }

# A stand-in for the teeny binary: records its arguments and exits.
stub="$scratch/teeny-stub"
cat >"$stub" <<'STUB'
#!/bin/sh
printf '%s\n' "$*" >"$STUB_OUT"
STUB
chmod +x "$stub"

run_entrypoint() {
  TEENY_DATA="$1" TEENY_BIN="$stub" STUB_OUT="$scratch/args" "$entrypoint" "${@:2}"
}

# --- a volume the legacy service wrote: everything flat at the root ----------
legacy="$scratch/legacy"
mkdir -p "$legacy/charts" "$legacy/moon" "$legacy/likeness" "$legacy/backrooms" \
  "$legacy/media" "$legacy/articles" "$legacy/memory/history"
echo '{"cursor":"1"}' >"$legacy/state.json"
echo '[]' >"$legacy/quotes.json"
echo '{"episode":3}' >"$legacy/narrative-state.json"
echo png >"$legacy/charts/2026-09-30.png"
echo png >"$legacy/moon/2026-09-29.png"
echo jpg >"$legacy/likeness/face.jpg"
echo '{}' >"$legacy/backrooms/run-1.json"
echo mp4 >"$legacy/media/morning.mp4"
echo '{"sheet":1}' >"$legacy/media/reference-sheet.json"
echo md >"$legacy/articles/a.md"
echo '# Learnings' >"$legacy/memory/LEARNINGS.md"

run_entrypoint "$legacy" serve

for dir in openhuman chrome-profile media articles state; do
  check "creates /data/$dir" test -d "$legacy/$dir"
done
check "creates a home under the OpenHuman workspace" test -d "$legacy/openhuman/home"
check "copies state.json into state/" grep -qs '"cursor":"1"' "$legacy/state/state.json"
check "copies quotes.json into state/" test -f "$legacy/state/quotes.json"
check "copies narrative-state.json into state/" grep -qs episode "$legacy/state/narrative-state.json"
check "copies backrooms/ into state/" test -f "$legacy/state/backrooms/run-1.json"
check "copies the learnings brief into state/memory/" grep -qs Learnings "$legacy/state/memory/LEARNINGS.md"
check "copies charts/ into media/" test -f "$legacy/media/charts/2026-09-30.png"
check "copies moon/ into media/" test -f "$legacy/media/moon/2026-09-29.png"
check "copies likeness/ into media/" test -f "$legacy/media/likeness/face.jpg"
check "leaves media/ and its reference sheet where they are" grep -qs sheet "$legacy/media/reference-sheet.json"
check "leaves articles/ where it is" test -f "$legacy/articles/a.md"
check "keeps the legacy files for a rollback" test -f "$legacy/state.json"
check "keeps the legacy dirs for a rollback" test -f "$legacy/charts/2026-09-30.png"
check "leaves no half-copied temporaries" \
  test -z "$(find "$legacy" -name '*.migrating*' -print)"
check "execs teeny with its arguments" grep -qsx serve "$scratch/args"

# --- idempotent: a second boot copies nothing over newer state ---------------
echo '{"cursor":"2"}' >"$legacy/state/state.json"
echo '{"cursor":"stale"}' >"$legacy/state.json"
run_entrypoint "$legacy" serve
check "never overwrites migrated state on a later boot" grep -qs '"cursor":"2"' "$legacy/state/state.json"

# --- a reference sheet the legacy layout left at the root ------------------
flat="$scratch/flat"
mkdir -p "$flat"
echo '{"sheet":"root"}' >"$flat/reference-sheet.json"
run_entrypoint "$flat" serve
check "copies a root reference sheet under media/" grep -qs root "$flat/media/reference-sheet.json"

# --- a fresh volume ---------------------------------------------------------
fresh="$scratch/fresh"
mkdir -p "$fresh"
run_entrypoint "$fresh" healthcheck
check "boots on an empty volume" test -d "$fresh/state"
check "passes any subcommand through" grep -qsx healthcheck "$scratch/args"

# --- stale Chrome singleton locks ------------------------------------------
locked="$scratch/locked"
mkdir -p "$locked/chrome-profile/Default"
ln -s "old-container-42" "$locked/chrome-profile/SingletonLock"
ln -s "/tmp/gone/SingletonSocket" "$locked/chrome-profile/SingletonSocket"
ln -s "12345" "$locked/chrome-profile/SingletonCookie"
echo keep >"$locked/chrome-profile/Default/Cookies"
run_entrypoint "$locked" serve
check "clears stale Singleton locks when no chromium runs" \
  test -z "$(find "$locked/chrome-profile" -name 'Singleton*' -print)"
check "keeps the rest of the profile" test -f "$locked/chrome-profile/Default/Cookies"

if [ "$failures" -ne 0 ]; then
  printf '\n%d failed\n' "$failures" >&2
  exit 1
fi
printf '\nall passed\n'
