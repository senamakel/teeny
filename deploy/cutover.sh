#!/usr/bin/env bash
# Move the live bot from the TypeScript service to the Rust one, once.
#
#   ./deploy/cutover.sh              # say what would happen, change nothing
#   ./deploy/cutover.sh --go         # do it
#   ./deploy/cutover.sh --undo       # put the TypeScript service back
#
# Run it after the shadow run (deploy/README.md, "Moving to the Rust bot")
# has shown the Rust bot's drafts are right. Both services answer as the same
# account off the same queue, so the order matters and so does the undo:
#
#  - the legacy bot STOPS before the Rust one starts. Two agents on one
#    account means two answers to one mention, in public;
#  - the bot-data volume is snapshotted before the Rust bot first touches it,
#    and the Rust entrypoint COPIES the flat legacy files into its layout
#    rather than moving them, so the legacy service can be started again on
#    the same volume and find its own state where it left it;
#  - the legacy image is kept, as teeny-legacy:current, the exact image that
#    was running, so --undo needs no build.
#
# The webhook needs nothing: nginx proxies X's deliveries to 127.0.0.1:BOT_PORT
# and the Rust bot takes that port.
#
# The previous cutover (teeny out of the stonks.rocks stack, 2026-09-08) was
# this script's first job; that version is in git history:
#   git log --follow -p -- deploy/cutover.sh

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

DEPLOY_HOST="${DEPLOY_HOST:-droid@174.138.35.76}"
DEPLOY_KEY="${DEPLOY_KEY:-$HOME/.ssh/id_rsa}"
REMOTE_DIR="${REMOTE_DIR:-/home/droid/teeny}"
PROJECT=teeny
VHOST=deploy/nginx/teeny.enamakel.com.conf
# The legacy image ran as uid 1001 (legacy/Dockerfile). The Rust entrypoint
# hands media/ and articles/ -- the two directories both layouts share -- to
# uid 10001; --undo hands them back.
LEGACY_UID=1001

GO=0
UNDO=0
for arg in "$@"; do
  case "$arg" in
    --go) GO=1 ;;
    --undo) UNDO=1 ;;
    -h|--help) sed -n '2,6p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) echo "unknown argument: $arg" >&2; exit 2 ;;
  esac
done

say() { printf '\n\033[1;35m==>\033[0m %s\n' "$*"; }
die() { printf '\n\033[1;31mfailed:\033[0m %s\n' "$*" >&2; exit 1; }
remote() { ssh -o BatchMode=yes -i "$DEPLOY_KEY" "$DEPLOY_HOST" "$@"; }
envval() { sed -n "s/^$1=//p" "$2" 2>/dev/null | tail -1; }

BOT_PORT="$(envval BOT_PORT deploy/.env)"; BOT_PORT="${BOT_PORT:-3110}"
LADDER="$(envval LADDER deploy/.env)"; LADDER="${LADDER:-ladder}"
LADDER_URL="$(envval LADDER_URL deploy/.env)"; LADDER_URL="${LADDER_URL:-http://$LADDER:6969/v1}"
COMPOSE="docker compose --project-directory $REMOTE_DIR/deploy -f $REMOTE_DIR/deploy/compose.prod.yaml --env-file $REMOTE_DIR/deploy/.env"
VOLUME="${PROJECT}_bot-data"
SNAPSHOT="${PROJECT}_bot-data-legacy"
RUNTIME_LABEL='{{index .Config.Labels "ai.tinyhumans.teeny.runtime"}}'

# `rust`, `legacy` or `none`, for a container or an image.
runtime_of() {
  local label
  label="$(remote "docker inspect --format '$RUNTIME_LABEL' $1 2>/dev/null")" || { echo none; return; }
  [ "$label" = rust ] && echo rust || echo legacy
}
running() { [ -n "$(remote "docker ps -q --filter name=^$1\$")" ]; }
wait_healthy() {
  for _ in $(seq 1 90); do
    remote "curl -fsS --max-time 3 -o /dev/null http://127.0.0.1:$1/health" && return 0
    sleep 1
  done
  return 1
}

[ -f "$DEPLOY_KEY" ] || die "no SSH key at $DEPLOY_KEY (override with DEPLOY_KEY=...)"

if [ "$UNDO" -eq 1 ]; then
  [ "$(runtime_of "$PROJECT-legacy:current")" = legacy ] \
    || die "no $PROJECT-legacy:current on the box. Build it: $COMPOSE --profile legacy build legacy-bot"
  say "Stopping the Rust bot (the account goes quiet from here)"
  remote "$COMPOSE stop bot" || die "could not stop the Rust bot; refusing to start a second agent on the account"
  say "Handing media/ and articles/ back to the legacy uid"
  remote "docker run --rm -v $VOLUME:/data alpine:3 sh -c 'for d in media articles; do [ -d /data/\$d ] && chown -R $LEGACY_UID:$LEGACY_UID /data/\$d; done; true'" \
    || die "could not re-own the shared directories"
  say "Starting the legacy bot on :$BOT_PORT"
  # The webhook's port, for as long as legacy is the live one. The shell
  # environment wins over deploy/.env in compose's interpolation.
  remote "LEGACY_BOT_PORT=$BOT_PORT $COMPOSE --profile legacy up -d --no-build legacy-bot" \
    || die "legacy-bot did not start"
  wait_healthy "$BOT_PORT" || { remote "$COMPOSE --profile legacy logs --tail 60 legacy-bot" >&2; die "legacy-bot never answered on :$BOT_PORT"; }
  say "Back on the TypeScript service. The Rust bot's layout is still on the volume; ./deploy/cutover.sh --go returns to it."
  exit 0
fi

if [ "$GO" -eq 0 ]; then
  cat <<PLAN

This is a dry run. Nothing has been changed. With --go it would:

  1. keep the running legacy image as $PROJECT-legacy:current, for --undo;
  2. build the Rust image on the box (deploy.sh --build-only), so the
     account is quiet for seconds rather than for the length of a build;
  3. stop the legacy bot ($PROJECT-bot-1 or $PROJECT-legacy-bot-1)
     --- from here the account answers nothing until step 5;
  4. snapshot $VOLUME -> $SNAPSHOT (once; an existing snapshot is kept);
  5. start the Rust bot on $VOLUME and :$BOT_PORT, the webhook's port; its
     entrypoint copies the legacy state into /data/{state,media};
  6. prove it is healthy and that a container wired like it reaches the
     ladder at $LADDER_URL.

Nothing is deleted: the legacy files stay on the volume, the legacy
container is stopped, and ./deploy/cutover.sh --undo reverses it.

  ./deploy/cutover.sh --go

PLAN
  exit 0
fi

# --- preflight -------------------------------------------------------------
grep -q "127.0.0.1:$BOT_PORT" "$VHOST" \
  || die "BOT_PORT is $BOT_PORT but $VHOST proxies the webhook elsewhere. If deploy/.env still has the shadow run's port swap (BOT_PORT=3111, LEGACY_BOT_PORT=3110), remove it first."
[ "$(envval DRY_RUN .env)" = false ] \
  || say "DRY_RUN is not false in .env: the Rust bot will draft, not post. Fine for a rehearsal; not a cutover."

# --- 1. the legacy image ---------------------------------------------------
LEGACY_CONTAINER=""
if [ "$(runtime_of "$PROJECT-bot-1")" = legacy ]; then
  LEGACY_CONTAINER="$PROJECT-bot-1"
elif running "$PROJECT-legacy-bot-1"; then
  LEGACY_CONTAINER="$PROJECT-legacy-bot-1"
fi
if [ -n "$LEGACY_CONTAINER" ]; then
  say "Keeping the image $LEGACY_CONTAINER runs as $PROJECT-legacy:current"
  remote "docker tag \$(docker inspect --format '{{.Image}}' $LEGACY_CONTAINER) $PROJECT-legacy:current"
else
  say "No legacy bot is running; nothing to stop"
fi

# --- 2. the Rust image ------------------------------------------------------
say "Building the Rust image on the box"
./deploy/deploy.sh --build-only || die "build failed; nothing has been changed"
[ "$(runtime_of "$PROJECT:current")" = rust ] || die "$PROJECT:current on the box is not the Rust image"

# --- 3. silence the legacy bot ----------------------------------------------
if [ -n "$LEGACY_CONTAINER" ]; then
  say "Stopping $LEGACY_CONTAINER (the account goes quiet from here)"
  remote "docker stop $LEGACY_CONTAINER >/dev/null" \
    || die "could not stop the legacy bot; refusing to start a second agent on the account"
fi

# --- 4. the snapshot --------------------------------------------------------
if remote "docker volume inspect $SNAPSHOT >/dev/null 2>&1"; then
  say "Snapshot $SNAPSHOT already exists; keeping it"
else
  say "Snapshotting $VOLUME -> $SNAPSHOT"
  remote "docker volume create $SNAPSHOT >/dev/null && docker run --rm -v $VOLUME:/from:ro -v $SNAPSHOT:/to alpine:3 sh -c 'cd /from && cp -a . /to/'" \
    || die "snapshot failed; the legacy bot is stopped, run ./deploy/cutover.sh --undo"
fi

# --- 5. start the Rust bot ---------------------------------------------------
say "Starting the Rust bot"
remote "$COMPOSE up -d --no-build --remove-orphans" || die "compose up failed; run ./deploy/cutover.sh --undo"
wait_healthy "$BOT_PORT" \
  || { remote "$COMPOSE logs --tail 60 bot" >&2; die "the Rust bot never answered; run ./deploy/cutover.sh --undo"; }
say "The Rust bot answers on 127.0.0.1:$BOT_PORT"
remote "docker exec $PROJECT-bot-1 ls /data/state /data/media" || true

# --- 6. the ladder -----------------------------------------------------------
# The agent is useless without it and looks healthy without it. The Rust image
# has no shell tools to ask from inside, so ask from a container wired the
# same way, as deploy.sh does.
remote "docker run --rm --add-host $LADDER:host-gateway alpine:3 wget -q -O /dev/null -T 5 \
      --header 'authorization: Bearer $(envval LADDER_API_KEY deploy/.env)' '${LADDER_URL%/}/models'" \
  || die "the ladder does not answer at $LADDER_URL from a container here; run ./deploy/cutover.sh --undo"
say "The ladder answers at $LADDER_URL"

say "Cut over."
remote "curl -sS http://127.0.0.1:$BOT_PORT/queue" || true; echo
cat <<DONE

The legacy container is stopped, not removed, and its files are still on the
volume, with a full copy in $SNAPSHOT. Once the Rust bot has posted and
replied for a few days, the legacy service can go (deploy/README.md):

  ssh $DEPLOY_HOST 'docker rm $PROJECT-legacy-bot-1; docker volume rm $SNAPSHOT'

  ./deploy/cutover.sh --undo   # go back
DONE
