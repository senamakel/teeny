#!/bin/sh
# Runs the OpenCompany host over the brain checkout, and restarts it when
# asked to by file.
#
# A running host never re-reads company.toml: the in-place rebuild route reads
# the persisted manifest, and only a process restart loads a changed
# definition. The curator agent edits /data/company/teeny and needs the host
# to come back up over the edit -- without a docker socket anywhere, which is
# root on the box. So the contract is a file: anything that wants the new
# definition loaded touches /data/.reload-requested and stops. This loop sees
# it, removes it, stops the server, and starts it again. The brain-sync
# sidecar touches the same file after it pulls a definition change from
# GitHub, so a person editing on the web gets the same reload.
#
# If the server exits on its own -- a boot refusal, a crash -- this script
# exits with its status and compose's restart policy brings the container
# back, which is the right shape: a definition that fails to load is loud in
# `docker compose logs opencompany` rather than silently retried here.
set -eu

DATA="${OPENCOMPANY_DATA_DIR:-/data}"
COMPANY="${OPENCOMPANY_COMPANY_DIR:-$DATA/company/teeny}"
BIND="${OPENCOMPANY_BIND:-0.0.0.0:8080}"
FLAG="$DATA/.reload-requested"

if [ ! -f "$COMPANY/company.toml" ]; then
  echo "opencompany: no manifest at $COMPANY/company.toml; is the brain checkout mounted at $DATA?" >&2
  exit 1
fi

while :; do
  rm -f "$FLAG"
  echo "opencompany: serving $COMPANY on $BIND, home $DATA"
  opencompany serve --company "$COMPANY" --bind "$BIND" --home "$DATA" &
  pid=$!

  # Wait for either the reload flag or the server going away. `kill -0` is
  # the liveness probe; `wait` at the end collects the status.
  while kill -0 "$pid" 2>/dev/null; do
    if [ -e "$FLAG" ]; then
      echo "opencompany: reload requested; restarting over $COMPANY"
      kill -TERM "$pid" 2>/dev/null || true
      # Give it a moment to release the data-root lock cleanly, then insist.
      n=0
      while kill -0 "$pid" 2>/dev/null && [ "$n" -lt 30 ]; do sleep 1; n=$((n + 1)); done
      kill -KILL "$pid" 2>/dev/null || true
      wait "$pid" 2>/dev/null || true
      continue 2
    fi
    sleep 2
  done

  status=0
  wait "$pid" || status=$?
  echo "opencompany: server exited with status $status" >&2
  exit "$status"
done
