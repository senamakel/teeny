#!/usr/bin/env bash
# Boots a real OpenHuman core through bin/kernel over a scratch root and checks
# the whole step-1 surface: first-boot minting, the single storage file, the
# file keyring, cron seeded from workspace/cron.toml, a shell job that runs,
# bin/crons reading storage.db, bin/oh, and bin/ask completing a turn against
# a stub OpenAI-compatible model.
#
#   OPENHUMAN_CORE_BIN=/path/to/openhuman-core bin/test/kernel_live_test.sh
#
# Skips (exit 0) when no core binary is given; CI runs bin/test/seed_cron_test.sh
# instead. Build one from upstream/main with
# `cargo build --release -p openhuman-cli --bin openhuman-core`.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
if [ -z "${OPENHUMAN_CORE_BIN:-}" ]; then
  echo "skip: set OPENHUMAN_CORE_BIN to an openhuman-core binary to run this test"
  exit 0
fi

# bin/kernel runs from a scratch copy, so a relative path would not resolve.
OPENHUMAN_CORE_BIN="$(python3 -c 'import os, sys; print(os.path.abspath(sys.argv[1]))' "$OPENHUMAN_CORE_BIN")"
export OPENHUMAN_CORE_BIN
[ -x "$OPENHUMAN_CORE_BIN" ] || { echo "FAIL no executable at $OPENHUMAN_CORE_BIN" >&2; exit 1; }

scratch="$(mktemp -d "${TMPDIR:-/tmp}/teeny-kernel.XXXXXX")"
kernel_pid=""
model_pid=""
cleanup() {
  [ -n "$kernel_pid" ] && kill -TERM "$kernel_pid" 2>/dev/null && wait "$kernel_pid" 2>/dev/null
  [ -n "$model_pid" ] && kill "$model_pid" 2>/dev/null
  if [ "${KEEP_SCRATCH:-0}" = 1 ]; then echo "scratch kept at $scratch"; else rm -rf "$scratch"; fi
}
trap cleanup EXIT

failures=0
pass() { printf 'ok   %s\n' "$1"; }
fail() { printf 'FAIL %s\n' "$1" >&2; failures=$((failures + 1)); }
check() { local name="$1"; shift; if "$@"; then pass "$name"; else fail "$name"; fi; }
free_port() { python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])'; }

# --- a stub model: answers every chat completion with one fixed sentence -------
model_port="$(free_port)"
cat >"$scratch/model.py" <<'PY'
import json, sys, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

ANSWER = "teeny kernel says hello"

class Model(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def _json(self, body):
        data = json.dumps(body).encode()
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        self._json({"object": "list", "data": [{"id": "stub", "object": "model"}]})

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers.get("content-length", 0))) or b"{}")
        with open(sys.argv[2], "a") as log:
            log.write(self.path + "\n")
        base = {"id": "stub-1", "created": int(time.time()), "model": body.get("model", "stub")}
        if body.get("stream"):
            self.send_response(200)
            self.send_header("content-type", "text/event-stream")
            self.end_headers()
            chunks = [
                {**base, "object": "chat.completion.chunk",
                 "choices": [{"index": 0, "delta": {"role": "assistant", "content": ANSWER}, "finish_reason": None}]},
                {**base, "object": "chat.completion.chunk",
                 "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
                 "usage": {"prompt_tokens": 1, "completion_tokens": 5, "total_tokens": 6}},
            ]
            for chunk in chunks:
                self.wfile.write(f"data: {json.dumps(chunk)}\n\n".encode())
            self.wfile.write(b"data: [DONE]\n\n")
            return
        self._json({**base, "object": "chat.completion",
                    "choices": [{"index": 0, "message": {"role": "assistant", "content": ANSWER}, "finish_reason": "stop"}],
                    "usage": {"prompt_tokens": 1, "completion_tokens": 5, "total_tokens": 6}})

ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), Model).serve_forever()
PY
python3 -I "$scratch/model.py" "$model_port" "$scratch/model-calls.log" &
model_pid=$!

# --- a scratch copy of the checkout, laid out as on the box ---------------------
# Cron shell jobs get an allowlisted environment and run from the action dir,
# so a job finds state/ and secrets/ only from the root it runs in. Relocating
# those with TEENY_* variables would test something production never does.
root="$scratch/root"
mkdir -p "$root/workspace" "$root/secrets"
cp -R "$repo/bin" "$root/bin"
cp "$repo/workspace/cron.toml" "$root/workspace/cron.toml"
cp "$repo/workspace/SOUL.md" "$root/workspace/SOUL.md"
cat >"$root/secrets/kernel.env" <<ENV
LADDER_URL=http://127.0.0.1:$model_port/v1
LADDER_API_KEY=stub-key
KERNEL_MODEL=stub
ENV

core_port="$(free_port)"
export OPENHUMAN_CORE_PORT="$core_port" KERNEL_BOOT_SECS=90
unset OPENHUMAN_CORE_TOKEN OPENHUMAN_CORE_URL OPENHUMAN_WORKSPACE CALENDAR_TZ \
  TEENY_ROOT TEENY_WORKSPACE TEENY_STORAGE TEENY_SECRETS TEENY_STATE
"$root/bin/kernel" >"$scratch/kernel.log" 2>&1 &
kernel_pid=$!
for _ in $(seq 1 120); do
  grep -q '\[kernel\] ready' "$scratch/kernel.log" && break
  kill -0 "$kernel_pid" 2>/dev/null || break
  sleep 1
done
if ! grep -q '\[kernel\] ready' "$scratch/kernel.log"; then
  cat "$scratch/kernel.log" >&2
  echo "FAIL the kernel never became ready" >&2
  exit 1
fi
pass "the kernel boots and reports ready"

# --- first boot -----------------------------------------------------------------
check "mints the core token" test -s "$root/secrets/core.token"
check "mints the keyring master key" test -s "$root/secrets/openhuman.key"
check "keeps the token private" test "$(stat -c %a "$root/secrets/core.token" 2>/dev/null || stat -f %Lp "$root/secrets/core.token")" = 600
check "writes one storage file in the workspace" test -s "$root/workspace/storage.db"
check "leaves Teeny's SOUL.md exactly as it was" cmp -s "$repo/workspace/SOUL.md" "$root/workspace/SOUL.md"
check "keeps config out of the workspace" test ! -e "$root/workspace/config.toml"
check "puts config beside the workspace" test -s "$root/.openhuman/config.toml"
check "never exports the model key to the core" \
  bash -c "! grep -q stub-key /proc/$kernel_pid/environ 2>/dev/null"

# --- cron, seeded and visible as a query ----------------------------------------
crons="$("$root/bin/crons" 2>&1)"
check "bin/crons lists the seeded heartbeat" grep -q 'teeny.heartbeat' <<<"$crons"
check "bin/crons shows it as a shell job" grep -Eq 'teeny.heartbeat +shell' <<<"$crons"
check "bin/crons also shows the core's own jobs" grep -q 'memory_background' <<<"$crons"
check "bin/crons shows the job's timezone" grep -q 'teeny.heartbeat.*UTC' <<<"$crons"
check "bin/crons refuses a name that is not a job name" bash -c "! '$root/bin/crons' runs \"x'; drop table docs; --\" 2>/dev/null"

check "records the core URL for cron jobs" grep -q ":$core_port" "$root/state/core.url"
id="$("$root/bin/oh" cron_list | jq -r '.[] | select(.name == "teeny.heartbeat") | .id')"
check "bin/oh reads the job back over RPC" test -n "$id"
"$root/bin/oh" cron_run "$(jq -cn --arg id "$id" '{job_id: $id}')" >/dev/null
for _ in $(seq 1 30); do [ -s "$root/state/heartbeat" ] && break; sleep 1; done
check "the heartbeat job runs and writes state/heartbeat" test -s "$root/state/heartbeat"
sleep 1
check "bin/crons runs shows the run ok" grep -q ' ok ' <<<"$("$root/bin/crons" runs heartbeat 2>&1)"

TEENY_OH="$root/bin/oh" "$root/bin/seed-cron" 2>"$scratch/reseed.log"
check "re-seeding an unchanged file changes nothing" grep -q '0 change(s) applied' "$scratch/reseed.log"
sed -i.bak 's#\*/5 \* \* \* \*#*/10 * * * *#' "$root/workspace/cron.toml"
"$root/bin/seed-cron" 2>"$scratch/reseed.log"
check "a changed schedule is updated on the real core" \
  test "$("$root/bin/oh" cron_list | jq -r '.[] | select(.name == "teeny.heartbeat") | .schedule.expr')" = "*/10 * * * *"

# --- bin/oh errors, bin/ask a whole turn ----------------------------------------
if "$root/bin/oh" cron_remove '{"job_id":"no-such-job"}' >/dev/null 2>"$scratch/err.log" ||
   ! grep -q 'openhuman.cron_remove' "$scratch/err.log"; then
  # A core that treats removing an unknown id as a no-op is fine too; what must
  # not happen is an error swallowed into a zero exit.
  if grep -q 'openhuman.cron_remove' "$scratch/err.log"; then fail "bin/oh exits non-zero on an RPC error"; else pass "bin/oh: removing an unknown id is a no-op on this core"; fi
else
  pass "bin/oh exits non-zero on an RPC error and names the method"
fi
if OPENHUMAN_CORE_TOKEN=wrong "$root/bin/oh" cron_list >/dev/null 2>&1; then
  fail "bin/oh is refused with the wrong token"
else
  pass "bin/oh is refused with the wrong token"
fi

answer="$("$root/bin/ask" "say hello" 2>"$scratch/ask.log" || true)"
check "bin/ask completes a turn through the configured model" grep -q 'teeny kernel says hello' <<<"$answer"
check "the turn went to the configured endpoint" grep -q 'chat/completions' "$scratch/model-calls.log"

# --- the keyring survives a restart ---------------------------------------------
kill -TERM "$kernel_pid"
wait "$kernel_pid" 2>/dev/null || true
kernel_pid=""
check "the kernel stops on SIGTERM" bash -c "! curl -fsS -o /dev/null http://127.0.0.1:$core_port/health"
: >"$scratch/model-calls.log"
# A second boot with the key gone from kernel.env must still reach the model.
sed -i.bak '/^LADDER_API_KEY=/d' "$root/secrets/kernel.env"
"$root/bin/kernel" >"$scratch/kernel2.log" 2>&1 &
kernel_pid=$!
for _ in $(seq 1 120); do grep -q '\[kernel\] ready' "$scratch/kernel2.log" && break; sleep 1; done
answer="$("$root/bin/ask" "say hello" 2>/dev/null || true)"
check "the model key survives a restart in the encrypted store" grep -q 'teeny kernel says hello' <<<"$answer"

if [ "$failures" -gt 0 ]; then
  printf '%d failure(s); kernel log:\n' "$failures" >&2
  tail -40 "$scratch/kernel.log" >&2
  exit 1
fi
echo "all passed"
