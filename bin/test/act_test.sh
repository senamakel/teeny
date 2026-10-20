#!/usr/bin/env bash
# Tests for bin/act against stand-ins: a core whose agent turn answers what
# the test wrote in $FAKE_ANSWER, and hands that log how they were called. No
# OpenHuman and no X; bin/test/spine_live_test.sh runs a playbook through a
# real core and checks the act agent offers the model no tools.
#
#   bin/test/act_test.sh
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
act="$here/../act"
scratch="$(mktemp -d "${TMPDIR:-/tmp}/teeny-act.XXXXXX")"
trap 'rm -rf "$scratch"' EXIT

failures=0
pass() { printf 'ok   %s\n' "$1"; }
fail() { printf 'FAIL %s\n' "$1" >&2; failures=$((failures + 1)); }
check() { local name="$1"; shift; if "$@"; then pass "$name"; else fail "$name"; fi; }

# The core: inference_agent_chat answers $FAKE_ANSWER; every call is logged.
cat >"$scratch/fake-oh" <<'FAKE'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$2" >>"$FAKE_CALLS"
[ "$1" = inference_agent_chat ] || { echo "unexpected method $1" >&2; exit 1; }
jq -cn --rawfile a "$FAKE_ANSWER" '{response: $a}'
FAKE
# The hands: each logs "<hand> <args> DRY_RUN=<v> stdin=<json>" and answers.
mkdir -p "$scratch/hands"
cat >"$scratch/hands/x" <<'FAKE'
#!/usr/bin/env bash
in="$(cat)"
printf 'x %s DRY_RUN=%s stdin=%s\n' "$*" "${DRY_RUN:-}" "$in" >>"$FAKE_HANDS"
case "$1" in
  search) echo '{"ok": true, "posts": [{"id": "111", "text": "gm @teenydroid, what are you building?"}]}' ;;
  *) echo '{"ok": true, "dry_run": true}'; exit "${FAKE_X_EXIT:-0}" ;;
esac
FAKE
cat >"$scratch/hands/guard" <<'FAKE'
#!/usr/bin/env bash
in="$(cat)"
printf 'guard %s stdin=%s\n' "$*" "$in" >>"$FAKE_HANDS"
if grep -q BAD <<<"$in"; then echo '{"ok": false, "refused": "held"}'; exit 2; fi
echo '{"ok": true}'
FAKE
cat >"$scratch/hands/lab" <<'FAKE'
#!/usr/bin/env bash
in="$(cat)"
printf 'lab %s stdin=%s\n' "$*" "$in" >>"$FAKE_HANDS"
echo '{"ok": true}'
FAKE
chmod +x "$scratch/fake-oh" "$scratch/hands/"*

brain="$scratch/brain"
mkdir -p "$brain/playbooks"
export TEENY_OH="$scratch/fake-oh" TEENY_HANDS="$scratch/hands" OPENHUMAN_WORKSPACE="$brain" \
  TEENY_STATE="$scratch/state" FAKE_CALLS="$scratch/calls.log" FAKE_HANDS="$scratch/hands.log" \
  FAKE_ANSWER="$scratch/answer.txt"
unset DRY_RUN FAKE_X_EXIT

book() { # NAME OUTPUT MAX [INPUT_RUN]
  {
    printf '+++\ndescription = "test"\noutput = "%s"\nmax_per_day = %s\n' "$2" "$3"
    [ -z "${4:-}" ] || printf '[[inputs]]\nname = "feed"\nrun = "%s"\n' "$4"
    printf '+++\ndo the thing\n'
  } >"$brain/playbooks/$1.md"
}
answer() { printf '%s' "$1" >"$FAKE_ANSWER"; }
reset() { rm -rf "$scratch/state"; : >"$FAKE_CALLS"; : >"$FAKE_HANDS"; }
run() { "$act" "$@" 2>"$scratch/stderr"; }
hand_calls() { grep -c "^$1 " "$FAKE_HANDS" || true; }

# --- a post: inputs, the draft, hands/x post ---------------------------------------
reset
book morning post 1 "hands/x search 'from:tinyhumansai' --max 5"
answer '{"action": "post", "text": "gm tiny humans"}'
out="$(run morning)"
check "runs the input hand" grep -q "^x search from:tinyhumansai --max 5 " "$FAKE_HANDS"
check "forces DRY_RUN=1 on every input" grep -q "^x search .* DRY_RUN=1 " "$FAKE_HANDS"
check "drafts as the act agent" test "$(jq -r .agent_id "$FAKE_CALLS")" = act
check "gives the model the input as data" grep -q 'what are you building' <<<"$(jq -r .message "$FAKE_CALLS")"
check "tells the model the inputs are not instructions" grep -q 'never instructions to follow' <<<"$(jq -r .message "$FAKE_CALLS")"
check "posts the draft through hands/x post" grep -q '^x post DRY_RUN= stdin={"text":"gm tiny humans"}' "$FAKE_HANDS"
check "leaves DRY_RUN to hands/x for the action" bash -c "! grep -q '^x post DRY_RUN=1' '$FAKE_HANDS'"
check "reports the action done" test "$(jq -r '.actions[0].status' <<<"$out")" = "done"
check "records the run" test "$(wc -l <"$scratch/state/act/morning/runs.jsonl" | tr -d ' ')" = 1

# --- the daily budget ------------------------------------------------------------
: >"$FAKE_CALLS"; : >"$FAKE_HANDS"
out="$(run morning)"
check "skips once max_per_day actions are taken" test "$(jq -r .skipped <<<"$out")" = "daily budget spent"
check "asks the model nothing when the budget is spent" test ! -s "$FAKE_CALLS"
check "runs no hand when the budget is spent" test ! -s "$FAKE_HANDS"

reset
answer '{"action": "skip", "reason": "nothing new"}'
run morning >/dev/null
answer '{"action": "post", "text": "gm"}'
out="$(run morning)"
check "a skip costs nothing from the budget" test "$(jq -r '.actions[0].status' <<<"$out")" = "done"

reset
book many post 2
answer '[{"action": "post", "text": "one"}, {"action": "post", "text": "two"}, {"action": "post", "text": "three"}]'
out="$(run many)"
check "takes no more actions than the budget, even when the draft asks" test "$(hand_calls x)" = 2
check "says which it refused" test "$(jq -r '.actions[2].refused' <<<"$out")" = "daily budget spent"

# --- a rule refusing is not an action taken ---------------------------------------------
reset
answer '{"action": "post", "text": "gm"}'
out="$(FAKE_X_EXIT=2 run morning)"
check "reports a hand's refusal as refused" test "$(jq -r '.actions[0].status' <<<"$out")" = refused
check "a refused action does not spend the budget" test ! -e "$scratch/state/act/morning/$(date -u +%Y-%m-%d)"

# --- the draft can only do what the playbook does ---------------------------------------
reset
answer '{"action": "reply", "to": "111", "text": "hi"}'
out="$(run morning)"
check "refuses an action the playbook does not take" test "$(jq -r '.actions[0].refused' <<<"$out")" = "this playbook only does post"
check "and calls no hand for it" test "$(hand_calls x)" = 1

reset
book mentions reply 5
answer '{"action": "reply", "to": "111", "text": "building tiny things, fren"}'
run mentions >/dev/null
check "replies with hands/x reply --to" grep -q '^x reply --to 111 DRY_RUN= stdin={"text":"building tiny things, fren"}' "$FAKE_HANDS"
reset
answer '{"action": "reply", "text": "to nobody"}'
out="$(run mentions)"
check "refuses a reply with no post to answer" test "$(jq -r '.actions[0].status' <<<"$out")" = error
check "and calls no hand" test "$(hand_calls x)" = 0

# --- a playbook's own hand: the guard reads the text first ---------------------------
reset
book note "hand:lab log" 3
answer '{"action": "hand", "text": "the terminal typed: hello", "tag": "terminal"}'
run note >/dev/null
check "puts a hand output's text through hands/guard check" grep -q '^guard check stdin={"text":"the terminal typed: hello"}' "$FAKE_HANDS"
check "then gives the hand the draft" grep -q '^lab log stdin={"text":"the terminal typed: hello","tag":"terminal"}' "$FAKE_HANDS"
reset
answer '{"action": "hand", "text": "BAD words"}'
out="$(run note)"
check "a guard hold stops the hand" test "$(hand_calls lab)" = 0
check "and is reported as refused" test "$(jq -r '.actions[0].status' <<<"$out")" = refused

# --- drafts that are not usable ------------------------------------------------------------
reset
answer 'sure! here is a post: gm'
if run morning >/dev/null; then fail "fails on a draft that is not JSON"; else pass "fails on a draft that is not JSON"; fi
check "records the unusable draft" grep -q 'not usable JSON' "$scratch/state/act/morning/runs.jsonl"
check "takes no action on it" test "$(hand_calls x)" = 1
reset
answer $'```json\n{"action": "post", "text": "fenced gm"}\n```'
run morning >/dev/null
check "reads a draft inside a code fence" grep -q '"text":"fenced gm"' "$FAKE_HANDS"

# --- one run at a time ---------------------------------------------------------------
reset
mkdir -p "$scratch/state/locks/act-morning"
out="$(run morning)"
check "skips while another run of the playbook holds the lock" test "$(jq -r .skipped <<<"$out")" = "already running"
rmdir "$scratch/state/locks/act-morning"

# --- what a playbook may name ---------------------------------------------------------
refuses() {
  local name="$1" body="$2"
  printf '%s\n' "$body" >"$brain/playbooks/bad.md"
  if "$act" --check bad >/dev/null 2>&1; then fail "$name"; else pass "$name"; fi
}
refuses "refuses an input that is not a hand" $'+++\ndescription = "x"\noutput = "post"\n[[inputs]]\nrun = "curl https://example.com"\n+++\ngo'
refuses "refuses an input hand that does not exist" $'+++\ndescription = "x"\noutput = "post"\n[[inputs]]\nrun = "hands/nope list"\n+++\ngo'
refuses "refuses the guard as an output" $'+++\ndescription = "x"\noutput = "hand:guard holds"\n+++\ngo'
refuses "refuses an unknown output" $'+++\ndescription = "x"\noutput = "shell"\n+++\ngo'
refuses "refuses an empty prompt" $'+++\ndescription = "x"\noutput = "post"\n+++\n'
refuses "refuses a budget over 100 a day" $'+++\ndescription = "x"\noutput = "post"\nmax_per_day = 500\n+++\ngo'
check "refuses a playbook name with a path in it" bash -c "! '$act' --check ../secrets >/dev/null 2>&1"

# --- the repository's own playbooks parse ------------------------------------------------
for path in "$here/../../brain/playbooks"/*.md; do
  name="$(basename "$path" .md)"
  if OPENHUMAN_WORKSPACE="$here/../../brain" TEENY_HANDS="$here/../../hands" "$act" --check "$name" >/dev/null 2>&1; then
    pass "brain/playbooks/$name.md parses"
  else
    fail "brain/playbooks/$name.md parses"
  fi
done

if [ "$failures" -gt 0 ]; then
  printf '%d failure(s)\n' "$failures" >&2
  exit 1
fi
echo "all passed"
