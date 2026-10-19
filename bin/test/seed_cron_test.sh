#!/usr/bin/env bash
# Tests for bin/seed-cron against a stand-in for the core's cron RPC that keeps
# its jobs in a JSON file. No OpenHuman needed; bin/test/kernel_live_test.sh
# runs the same reconciliation against a real core.
#
#   bin/test/seed_cron_test.sh
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
seed="$here/../seed-cron"
scratch="$(mktemp -d "${TMPDIR:-/tmp}/teeny-seed-cron.XXXXXX")"
trap 'rm -rf "$scratch"' EXIT

failures=0
pass() { printf 'ok   %s\n' "$1"; }
fail() { printf 'FAIL %s\n' "$1" >&2; failures=$((failures + 1)); }
check() { local name="$1"; shift; if "$@"; then pass "$name"; else fail "$name"; fi; }

# The stand-in: cron_list / cron_add / cron_update / cron_remove over
# $FAKE_JOBS, every call appended to $FAKE_CALLS.
fake="$scratch/fake-oh"
cat >"$fake" <<'FAKE'
#!/usr/bin/env bash
set -euo pipefail
method="$1"; params="${2:-}"; [ -n "$params" ] || params='{}'
printf '%s %s\n' "$method" "$params" >>"$FAKE_CALLS"
jobs="$(cat "$FAKE_JOBS")"
case "$method" in
  cron_list) printf '%s\n' "$jobs" ;;
  cron_add)
    id="job-$(( $(jq length <<<"$jobs") + $(wc -l <"$FAKE_CALLS") ))"
    job="$(jq -c --arg id "$id" '{id: $id, name, job_type, command, schedule, enabled: true}' <<<"$params")"
    jq -c --argjson j "$job" '. + [$j]' <<<"$jobs" >"$FAKE_JOBS"
    printf '%s\n' "$job" ;;
  cron_update)
    jq -c --argjson p "$params" 'map(if .id == $p.job_id then . + $p.patch else . end)' <<<"$jobs" >"$FAKE_JOBS"
    echo '{}' ;;
  cron_remove)
    jq -c --argjson p "$params" 'map(select(.id != $p.job_id))' <<<"$jobs" >"$FAKE_JOBS"
    echo '{}' ;;
  *) echo "unexpected method $method" >&2; exit 1 ;;
esac
FAKE
chmod +x "$fake"

export TEENY_OH="$fake" FAKE_JOBS="$scratch/jobs.json" FAKE_CALLS="$scratch/calls.log"
unset CALENDAR_TZ
toml="$scratch/cron.toml"

reset() {
  # The core's own system job is always present and never ours to touch.
  echo '[{"id":"sys-1","name":"memory_background","job_type":"flow","command":"system:memory_background","schedule":{"kind":"every"},"enabled":true}]' >"$FAKE_JOBS"
  : >"$FAKE_CALLS"
}
seed() { "$seed" --file "$toml" "$@" 2>"$scratch/stderr"; }
jobs() { cat "$FAKE_JOBS"; }
calls() { grep -c "^$1 " "$FAKE_CALLS" || true; }
job_field() { jq -r --arg n "$1" ".[] | select(.name == \$n) | $2" "$FAKE_JOBS"; }

# --- a first seed adds every job, shell-typed and prefixed --------------------
reset
cat >"$toml" <<'TOML'
[jobs.heartbeat]
schedule = "*/5 * * * *"
command = "bin/heartbeat"

[jobs.morning]
schedule = "0 7 * * *"
command = "bin/job morning"
tz = "Asia/Kolkata"
TOML
seed
check "adds both jobs" test "$(calls cron_add)" = 2
check "names them teeny.<name>" test "$(jobs | jq '[.[].name] | sort | join(",")')" = '"memory_background,teeny.heartbeat,teeny.morning"'
check "makes them shell jobs" test "$(job_field teeny.heartbeat .job_type)" = shell
check "stores the cron expression" test "$(job_field teeny.morning .schedule.expr)" = "0 7 * * *"
check "keeps a job's own tz" test "$(job_field teeny.morning .schedule.tz)" = Asia/Kolkata
check "defaults the tz to UTC" test "$(job_field teeny.heartbeat .schedule.tz)" = UTC
check "asks for no delivery" grep -q '"delivery":{"mode":"none"}' "$FAKE_CALLS"
check "leaves the core's system job alone" test "$(job_field memory_background .id)" = sys-1

# --- a second seed over an unchanged file does nothing -----------------------
: >"$FAKE_CALLS"
seed
check "is idempotent: no add" test "$(calls cron_add)" = 0
check "is idempotent: no update" test "$(calls cron_update)" = 0
check "is idempotent: no remove" test "$(calls cron_remove)" = 0
check "says so" grep -q '0 change(s) applied' "$scratch/stderr"

# --- a changed job is updated in place, keeping its id -----------------------
id_before="$(job_field teeny.morning .id)"
sed -i.bak 's/0 7 \* \* \*/30 6 * * */' "$toml"
: >"$FAKE_CALLS"
seed
check "updates a changed schedule" test "$(job_field teeny.morning .schedule.expr)" = "30 6 * * *"
check "updates in place (same id)" test "$(job_field teeny.morning .id)" = "$id_before"
check "touches only the changed job" test "$(calls cron_update)" = 1

# --- a job dropped from the file is removed; the system job stays ------------
cat >"$toml" <<'TOML'
[jobs.heartbeat]
schedule = "*/5 * * * *"
command = "bin/heartbeat"
TOML
seed
check "removes a job no longer in the file" test -z "$(job_field teeny.morning .id)"
check "still leaves the system job" test "$(job_field memory_background .id)" = sys-1

# --- enabled = false: added, then switched off; flipping back re-enables -----
reset
cat >"$toml" <<'TOML'
[jobs.paused]
schedule = "0 * * * *"
command = "bin/heartbeat"
enabled = false
TOML
seed
check "adds a disabled job switched off" test "$(job_field teeny.paused .enabled)" = false
sed -i.bak 's/enabled = false/enabled = true/' "$toml"
seed
check "re-enables it when the file does" test "$(job_field teeny.paused .enabled)" = true

# --- duplicates of one name collapse to one -----------------------------------
reset
jq -c '. + [
  {id:"d1",name:"teeny.heartbeat",job_type:"shell",command:"bin/heartbeat",schedule:{kind:"cron",expr:"*/5 * * * *",tz:"UTC"},enabled:true},
  {id:"d2",name:"teeny.heartbeat",job_type:"shell",command:"bin/heartbeat",schedule:{kind:"cron",expr:"*/5 * * * *",tz:"UTC"},enabled:true}]' \
  "$FAKE_JOBS" >"$scratch/j" && mv "$scratch/j" "$FAKE_JOBS"
printf '[jobs.heartbeat]\nschedule = "*/5 * * * *"\ncommand = "bin/heartbeat"\n' >"$toml"
seed
check "keeps one of two same-named jobs" test "$(jobs | jq '[.[] | select(.name == "teeny.heartbeat")] | length')" = 1

# --- CALENDAR_TZ is the default when the file names none ---------------------
reset
CALENDAR_TZ=Europe/Berlin seed
check "falls back to CALENDAR_TZ" test "$(job_field teeny.heartbeat .schedule.tz)" = Europe/Berlin

# --- --dry-run plans without calling anything that writes ---------------------
reset
seed --dry-run
check "dry run adds nothing" test "$(calls cron_add)" = 0
check "dry run reports the plan" grep -q 'add.*teeny.heartbeat' "$scratch/stderr"

# --- the file is validated before the core is asked anything -----------------
refuses() {
  local name="$1" body="$2"
  reset
  printf '%s\n' "$body" >"$toml"
  if seed; then fail "$name"; elif [ "$(calls cron_add)" = 0 ]; then pass "$name"; else fail "$name (still wrote)"; fi
}
refuses "refuses a command outside bin/" $'[jobs.x]\nschedule = "* * * * *"\ncommand = "curl https://example.com"'
refuses "refuses a bin/ path that climbs out" $'[jobs.x]\nschedule = "* * * * *"\ncommand = "bin/../secrets/run"'
refuses "refuses a four-field schedule" $'[jobs.x]\nschedule = "* * * *"\ncommand = "bin/heartbeat"'
refuses "refuses an unknown job key" $'[jobs.x]\nschedule = "* * * * *"\ncommand = "bin/heartbeat"\nprompt = "post something"'
refuses "refuses an uppercase name" $'[jobs.X]\nschedule = "* * * * *"\ncommand = "bin/heartbeat"'
refuses "refuses a non-boolean enabled" $'[jobs.x]\nschedule = "* * * * *"\ncommand = "bin/heartbeat"\nenabled = "no"'

# --- the repository's own schedule file is valid ------------------------------
reset
"$seed" --dry-run --file "$here/../../workspace/cron.toml" 2>"$scratch/stderr"
check "workspace/cron.toml parses and plans" grep -q 'teeny.heartbeat' "$scratch/stderr"

if [ "$failures" -gt 0 ]; then
  printf '%d failure(s)\n' "$failures" >&2
  exit 1
fi
echo "all passed"
