#!/usr/bin/env bash
# Ship teeny to the box.
#
#   ./deploy/deploy.sh                # check, sync, build, release, health check
#   ./deploy/deploy.sh --skip-checks  # skip the local lint/typecheck/test pass
#   ./deploy/deploy.sh --logs         # tail the running stack and exit
#   ./deploy/deploy.sh --down         # stop the stack (volumes are kept)
#   ./deploy/deploy.sh --rollback     # put the previous image back
#
# The image is tagged with the current commit and aliased to :current, and the
# outgoing alias is kept as :previous, so a failed health check can put the old
# container back without a rebuild.
#
# Two things a compose file cannot express, and the reason this script exists:
#
#  1. The ladder is a container this project does not own. It publishes on the
#     host's loopback only, so `host.docker.internal` cannot reach it from a
#     container — it has to be joined to this project's network for
#     `http://ladder:6969` to resolve, and compose cannot adopt a container it
#     did not create. Set LADDER_URL in deploy/.env instead when the ladder
#     lives on another box.
#  2. .env files are excluded from the rsync and copied explicitly, so a
#     missing local file can never wipe a working one on the box.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

DEPLOY_HOST="${DEPLOY_HOST:-root@168.100.10.208}"
DEPLOY_KEY="${DEPLOY_KEY:-$REPO_ROOT/key.pem}"
REMOTE_DIR="${REMOTE_DIR:-/root/teeny}"
PROJECT="teeny"
NETWORK="${NETWORK:-${PROJECT}_default}"
LADDER="${LADDER:-ladder}"

SKIP_CHECKS=0
ROLLBACK=0
LOGS=0
DOWN=0
for arg in "$@"; do
  case "$arg" in
    --skip-checks) SKIP_CHECKS=1 ;;
    --rollback) ROLLBACK=1 ;;
    --logs) LOGS=1 ;;
    --down) DOWN=1 ;;
    -h|--help) sed -n '2,10p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) echo "unknown argument: $arg" >&2; exit 2 ;;
  esac
done

say() { printf '\n\033[1;35m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33mwarning:\033[0m %s\n' "$*" >&2; }
die() { printf '\n\033[1;31mfailed:\033[0m %s\n' "$*" >&2; exit 1; }
remote() { ssh -o BatchMode=yes -i "$DEPLOY_KEY" "$DEPLOY_HOST" "$@"; }
# Read one value out of an env file. These are not shell scripts -- a value
# with a space or a `<` in it makes `set -a; . ./.env` die -- so never source
# them.
envval() { sed -n "s/^$1=//p" "$2" 2>/dev/null | tail -1; }

COMPOSE="docker compose --project-directory $REMOTE_DIR/deploy -f $REMOTE_DIR/deploy/compose.prod.yaml --env-file $REMOTE_DIR/deploy/.env"
BOT_PORT="${BOT_PORT:-$(envval BOT_PORT deploy/.env)}"
BOT_PORT="${BOT_PORT:-3110}"

[ -f "$DEPLOY_KEY" ] || die "no SSH key at $DEPLOY_KEY (override with DEPLOY_KEY=...)"

if [ "$LOGS" -eq 1 ]; then
  exec ssh -i "$DEPLOY_KEY" -t "$DEPLOY_HOST" "$COMPOSE logs -f --tail 100"
fi

if [ "$DOWN" -eq 1 ]; then
  say "Stopping the stack on $DEPLOY_HOST (volumes are kept)"
  remote "$COMPOSE down"
  exit 0
fi

if [ "$ROLLBACK" -eq 1 ]; then
  say "Rolling back to the previous image"
  remote "docker image inspect $PROJECT:previous >/dev/null 2>&1" \
    || die "no $PROJECT:previous on the box, nothing to roll back to"
  remote "docker tag $PROJECT:previous $PROJECT:current && $COMPOSE up -d --no-build bot bot-reply"
  say "Rolled back. Check: ./deploy/deploy.sh --logs"
  exit 0
fi

# --- preflight -------------------------------------------------------------
[ -f deploy/.env ] || die "deploy/.env is missing. Copy deploy/.env.example and fill it in."
[ -f .env ] || die ".env is missing. Copy .env.example and fill it in (the X and agent credentials)."

for required in MONGO_ROOT_PASSWORD CORTEX_API_KEY LADDER_API_KEY; do
  [ -n "$(envval "$required" deploy/.env)" ] || die "$required is empty in deploy/.env"
done
for required in X_API_KEY X_API_SECRET X_ACCESS_TOKEN X_ACCESS_TOKEN_SECRET X_BOT_USERNAME; do
  [ -n "$(envval "$required" .env)" ] || die "$required is empty in .env; the agent cannot post without it"
done

# CortexDB authenticates the agent with the key it was started with, so the two
# files have to agree. They live apart because one is the service's own key and
# the other is the client's copy of it, and a mismatch fails at the first
# recall rather than at boot.
if [ "$(envval CORTEX_API_KEY deploy/.env)" != "$(envval CORTEX_TOKEN .env)" ]; then
  die "CORTEX_API_KEY in deploy/.env and CORTEX_TOKEN in .env differ; CortexDB would refuse every call"
fi

# DRY_RUN is the only thing standing between a draft and a published post, so
# say out loud which one is being deployed.
if [ "$(envval DRY_RUN .env)" = "false" ]; then
  say "DRY_RUN=false — this deploy PUBLISHES replies as @$(envval X_BOT_USERNAME .env)"
else
  warn "DRY_RUN is not false: replies will be drafted and logged, never posted."
fi

say "Checking the box"
remote "command -v docker >/dev/null" || die "docker is not installed on $DEPLOY_HOST"
remote "docker compose version >/dev/null" || die "docker compose v2 is not available on $DEPLOY_HOST"

if [ "$SKIP_CHECKS" -eq 0 ]; then
  say "Running local checks"
  [ -d node_modules ] || npm ci
  npm run lint --silent || die "lint failed"
  npm run typecheck --silent || die "typecheck failed"
  npm test --silent || die "tests failed"
fi

GIT_SHA="$(git rev-parse --short HEAD 2>/dev/null || echo nogit)"
if ! git diff --quiet 2>/dev/null || ! git diff --cached --quiet 2>/dev/null; then
  GIT_SHA="${GIT_SHA}-dirty"
  say "Working tree is dirty; deploying as $GIT_SHA"
fi

# --- sync ------------------------------------------------------------------
say "Syncing source to $DEPLOY_HOST:$REMOTE_DIR"
remote "mkdir -p $REMOTE_DIR/deploy"
rsync -az --delete \
  -e "ssh -i $DEPLOY_KEY" \
  --exclude '.git' \
  --exclude 'node_modules' \
  --exclude 'dist' \
  --exclude 'data' \
  --exclude 'worktrees' \
  --exclude 'key.pem' \
  --exclude 'key.pem.pub' \
  --exclude '.env' \
  --exclude '.env.local' \
  ./ "$DEPLOY_HOST:$REMOTE_DIR/"

scp -q -i "$DEPLOY_KEY" .env "$DEPLOY_HOST:$REMOTE_DIR/.env"
scp -q -i "$DEPLOY_KEY" deploy/.env "$DEPLOY_HOST:$REMOTE_DIR/deploy/.env"

# --- the model endpoint ----------------------------------------------------
# Without one, the agent boots and then fails every turn, which looks like a
# healthy deploy from the outside. Prove it is reachable before releasing.
LADDER_URL="$(envval LADDER_URL deploy/.env)"
say "Checking the model endpoint"
remote "docker network inspect $NETWORK >/dev/null 2>&1 || docker network create $NETWORK" >/dev/null
if [ -z "$LADDER_URL" ] || [ "$LADDER_URL" = "http://$LADDER:6969/v1" ]; then
  if remote "docker inspect $LADDER >/dev/null 2>&1"; then
    remote "docker network connect $NETWORK $LADDER 2>/dev/null || true"
    say "Joined the '$LADDER' container to $NETWORK"
  else
    die "no container named '$LADDER' on $DEPLOY_HOST and no LADDER_URL in deploy/.env.
       The agent has no model endpoint, so every reply and every memory write
       would fail. Either start the ladder on this box, or point LADDER_URL at
       the box that runs it (it must be reachable from a container here)."
  fi
else
  remote "curl -fsS --max-time 5 -o /dev/null -H 'authorization: Bearer $(envval LADDER_API_KEY deploy/.env)' ${LADDER_URL%/}/models" \
    || die "the box cannot reach LADDER_URL ($LADDER_URL); fix that before deploying"
  say "Reached the ladder at $LADDER_URL"
fi

# --- build -----------------------------------------------------------------
say "Building the image on the box ($GIT_SHA)"
remote "$COMPOSE build" || die "image build failed"

# Keep the outgoing image so a bad deploy can be undone in one command.
remote "docker image inspect $PROJECT:current >/dev/null 2>&1 && docker tag $PROJECT:current $PROJECT:previous || true"
remote "docker tag $PROJECT:current $PROJECT:$GIT_SHA || true"

# --- release ---------------------------------------------------------------
say "Starting the stack"
remote "$COMPOSE up -d --remove-orphans" || die "compose up failed"

say "Waiting for the agent to answer"
for attempt in $(seq 1 60); do
  if remote "curl -fsS --max-time 3 http://127.0.0.1:$BOT_PORT/health >/dev/null"; then
    say "Healthy after ${attempt}s"
    remote "curl -sS http://127.0.0.1:$BOT_PORT/queue" || true; echo
    say "Deployed $GIT_SHA. Logs: ./deploy/deploy.sh --logs   Undo: ./deploy/deploy.sh --rollback"
    exit 0
  fi
  sleep 1
done

printf '\n\033[1;31mHealth check never passed. Recent logs:\033[0m\n' >&2
remote "$COMPOSE logs --tail 60 bot" >&2 || true
die "deploy did not come up; run ./deploy/deploy.sh --rollback to restore the previous image"
