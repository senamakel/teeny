#!/usr/bin/env bash
# Ship teeny to the box.
#
#   ./deploy/deploy.sh                # check, sync, build, release, health check
#   ./deploy/deploy.sh --skip-checks  # skip the local lint/typecheck/test pass
#   ./deploy/deploy.sh --build-only   # check, sync and build; start nothing
#   ./deploy/deploy.sh --logs         # tail the running stack and exit
#   ./deploy/deploy.sh --down         # stop the stack (volumes are kept)
#   ./deploy/deploy.sh --rollback     # put the previous image back
#   ./deploy/deploy.sh --keep-env     # leave the box's .env files as they are
#
# --keep-env is what CI uses. The two .env files are secrets and live on the
# box already; a deploy from GitHub Actions has neither and must not need
# them, so in this mode the preflight reads them from the box over SSH, the
# copy step is skipped, and nothing about the running configuration changes
# except the code.
#
# The image is tagged with the current commit and aliased to :current, and the
# outgoing alias is kept as :previous, so a failed health check can put the old
# container back without a rebuild.
#
# Two things a compose file cannot express, and the reason this script exists:
#
#  1. The ladder is a container this project does not own. Compose reaches it
#     through `extra_hosts` (`ladder` -> host-gateway, the docker0 address it
#     publishes on), which is static and survives the ladder being recreated;
#     what a compose file cannot do is prove that the address answers. This
#     script does, from a throwaway container wired exactly like the bot,
#     before anything is released. Set LADDER_URL in deploy/.env instead when
#     the ladder lives on another box.
#  2. .env files are excluded from the rsync and copied explicitly, so a
#     missing local file can never wipe a working one on the box.
#  3. The brain checkout (BRAIN_DIR) is a live git working tree the
#     OpenCompany host writes into and brain-sync commits from. It is cloned
#     once, outside the rsynced tree, and never synced or deleted from here;
#     the OpenCompany image is pulled, and the definition is checked before
#     the stack is started so a broken self-edit fails the deploy, not the
#     boot. Only brain/sync -- the sidecar's image, pinned by the submodule --
#     is rsynced, so the sidecar is always built from a reviewed copy.
#  4. The SSH key brain-sync pushes with is the teenydroid account's key,
#     generated on the box and never leaving it; a person adds the public
#     half to the account once, and this script says whether GitHub knows it.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

DEPLOY_HOST="${DEPLOY_HOST:-droid@174.138.35.76}"
DEPLOY_KEY="${DEPLOY_KEY:-$HOME/.ssh/id_rsa}"
REMOTE_DIR="${REMOTE_DIR:-/home/droid/teeny}"
PROJECT="teeny"
LADDER="${LADDER:-ladder}"

SKIP_CHECKS=0
ROLLBACK=0
LOGS=0
DOWN=0
BUILD_ONLY=0
KEEP_ENV=0
for arg in "$@"; do
  case "$arg" in
    --skip-checks) SKIP_CHECKS=1 ;;
    --keep-env) KEEP_ENV=1 ;;
    --build-only) BUILD_ONLY=1 ;;
    --rollback) ROLLBACK=1 ;;
    --logs) LOGS=1 ;;
    --down) DOWN=1 ;;
    -h|--help) sed -n '2,12p' "${BASH_SOURCE[0]}"; exit 0 ;;
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
# The same, from the copy on the box. Used for every read under --keep-env,
# where the local files may not exist at all.
remoteval() { remote "sed -n 's/^$1=//p' $REMOTE_DIR/$2 2>/dev/null | tail -1"; }
# One value, from wherever this deploy's configuration lives.
cfg() { if [ "$KEEP_ENV" -eq 1 ]; then remoteval "$@"; else envval "$@"; fi; }

COMPOSE="docker compose --project-directory $REMOTE_DIR/deploy -f $REMOTE_DIR/deploy/compose.prod.yaml --env-file $REMOTE_DIR/deploy/.env"
BOT_PORT="${BOT_PORT:-$(cfg BOT_PORT deploy/.env)}"
BOT_PORT="${BOT_PORT:-3110}"
WEB_PORT="${WEB_PORT:-$(cfg WEB_PORT deploy/.env)}"
WEB_PORT="${WEB_PORT:-3120}"

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
  remote "docker tag $PROJECT:previous $PROJECT:current && $COMPOSE up -d --no-build bot"
  remote "docker image inspect $PROJECT-web:previous >/dev/null 2>&1 && docker tag $PROJECT-web:previous $PROJECT-web:current && $COMPOSE up -d --no-build web" || true
  say "Rolled back. Check: ./deploy/deploy.sh --logs"
  exit 0
fi

# --- preflight -------------------------------------------------------------
if [ "$KEEP_ENV" -eq 1 ]; then
  say "Keeping the box's .env files (--keep-env)"
  remote "test -f $REMOTE_DIR/deploy/.env && test -f $REMOTE_DIR/.env" \
    || die "the box has no $REMOTE_DIR/.env or $REMOTE_DIR/deploy/.env; do one deploy from a machine that has them first"
else
  [ -f deploy/.env ] || die "deploy/.env is missing. Copy deploy/.env.example and fill it in."
  [ -f .env ] || die ".env is missing. Copy .env.example and fill it in (the X and agent credentials)."
fi

for required in MONGO_ROOT_PASSWORD CORTEX_API_KEY LADDER_API_KEY OPENCOMPANY_IMAGE BRAIN_ADMIN_EMAIL; do
  [ -n "$(cfg "$required" deploy/.env)" ] || die "$required is empty in deploy/.env"
done
BRAIN_DIR="$(cfg BRAIN_DIR deploy/.env)"; BRAIN_DIR="${BRAIN_DIR:-/home/droid/brain}"
BRAIN_REMOTE="$(cfg BRAIN_REMOTE deploy/.env)"; BRAIN_REMOTE="${BRAIN_REMOTE:-git@github.com:teenydroid/brain.git}"
BRAIN_SSH_KEY="$(cfg BRAIN_SSH_KEY deploy/.env)"; BRAIN_SSH_KEY="${BRAIN_SSH_KEY:-/home/droid/.ssh/teenydroid_ed25519}"
BRAIN_PORT="$(cfg BRAIN_PORT deploy/.env)"; BRAIN_PORT="${BRAIN_PORT:-3130}"
OPENCOMPANY_IMAGE="$(cfg OPENCOMPANY_IMAGE deploy/.env)"
for required in X_API_KEY X_API_SECRET X_ACCESS_TOKEN X_ACCESS_TOKEN_SECRET X_BOT_USERNAME; do
  [ -n "$(cfg "$required" .env)" ] || die "$required is empty in .env; the agent cannot post without it"
done

# CortexDB authenticates the agent with the key it was started with, so the two
# files have to agree. They live apart because one is the service's own key and
# the other is the client's copy of it, and a mismatch fails at the first
# recall rather than at boot.
if [ "$(cfg CORTEX_API_KEY deploy/.env)" != "$(cfg CORTEX_TOKEN .env)" ]; then
  die "CORTEX_API_KEY in deploy/.env and CORTEX_TOKEN in .env differ; CortexDB would refuse every call"
fi

# DRY_RUN is the only thing standing between a draft and a published post, so
# say out loud which one is being deployed.
if [ "$(cfg DRY_RUN .env)" = "false" ]; then
  say "DRY_RUN=false — this deploy PUBLISHES replies as @$(cfg X_BOT_USERNAME .env)"
else
  warn "DRY_RUN is not false: replies will be drafted and logged, never posted."
fi

# Telegram is a second public face of the account; say whether it is going up.
if [ -n "$(envval TELEGRAM_BOT_TOKEN .env)" ]; then
  if [ "$(envval TELEGRAM_DRY_RUN .env)" = "true" ]; then
    warn "TELEGRAM_BOT_TOKEN is set with TELEGRAM_DRY_RUN=true: Telegram replies are drafted and logged, never sent."
  else
    say "TELEGRAM_BOT_TOKEN is set — this deploy ANSWERS on Telegram"
  fi
else
  warn "TELEGRAM_BOT_TOKEN is empty: the account will not be on Telegram."
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
  --include '/brain/sync/***' \
  --exclude '/brain/*' \
  --exclude 'web/node_modules' \
  --exclude 'web/.next' \
  --exclude 'key.pem' \
  --exclude 'key.pem.pub' \
  --exclude '.env' \
  --exclude '.env.local' \
  ./ "$DEPLOY_HOST:$REMOTE_DIR/"

if [ "$KEEP_ENV" -eq 0 ]; then
  scp -q -i "$DEPLOY_KEY" .env "$DEPLOY_HOST:$REMOTE_DIR/.env"
  scp -q -i "$DEPLOY_KEY" deploy/.env "$DEPLOY_HOST:$REMOTE_DIR/deploy/.env"
fi

# --- the model endpoint ----------------------------------------------------
# Without one, the agent boots and then fails every turn, which looks like a
# healthy deploy from the outside: the container is healthy, the page is up,
# and the morning post and the chart just never appear. Prove it is reachable
# before releasing -- and prove it the way the bot will reach it, from a
# container with the same extra_hosts, not from the host's shell. The ladder
# used to be joined to the compose network at deploy time instead; a
# `ladder-up --pull` between deploys recreated it off the network and every
# scheduled post failed for three days (2026-09-15..18) before anyone looked.
LADDER_URL="$(cfg LADDER_URL deploy/.env)"
LADDER_URL="${LADDER_URL:-http://$LADDER:6969/v1}"
say "Checking the model endpoint"
if ! remote "docker run --rm --add-host $LADDER:host-gateway alpine:3 \
      wget -q -O /dev/null -T 5 --header 'authorization: Bearer $(cfg LADDER_API_KEY deploy/.env)' \
      '${LADDER_URL%/}/models'" >/dev/null 2>&1; then
  die "a container on $DEPLOY_HOST cannot reach the ladder at $LADDER_URL.
       The agent has no model endpoint, so every reply, every post and every
       memory write would fail. On this box the ladder must publish on the
       docker0 address, 172.17.0.1:6969, which is what '$LADDER' resolves to
       inside the stack (\`docker port $LADDER\` shows the bindings); on another
       box, point LADDER_URL at a URL reachable from a container here."
fi
say "Reached the ladder at $LADDER_URL"

# --- the brain -------------------------------------------------------------
# The checkout is live state: cloned once, then owned by the containers.
# Never rsynced, never reset from here. The account key that pushes is what
# brain-sync needs; the clone uses the same key so a missing one fails now.
say "Checking the brain"
# The key is the teenydroid GitHub account's SSH key, minted on the box and
# never leaving it. brain-sync pushes with it, passed explicitly; the box's
# own keys, its ~/.ssh/config and its gh login stay as they are, and only
# the opt-in alias git@github-teenydroid: uses it. Adding the public half to the
# account is a one-time step for a person, on
# https://github.com/settings/keys as teenydroid; this prints it until that
# has happened. brain-sync keeps its commits and retries the push every
# tick, so an unregistered key is not a failed deploy.
if ! remote "test -r $BRAIN_SSH_KEY"; then
  say "Generating the teenydroid SSH key at $BRAIN_SSH_KEY on the box"
  remote "mkdir -p \$(dirname $BRAIN_SSH_KEY) && ssh-keygen -q -t ed25519 -N '' -C 'teenydroid@robot1' -f $BRAIN_SSH_KEY" \
    || die "could not generate $BRAIN_SSH_KEY on the box"
fi
if remote "ssh -i $BRAIN_SSH_KEY -o IdentitiesOnly=yes -o BatchMode=yes -o StrictHostKeyChecking=accept-new -T git@github.com 2>&1 | grep -q 'successfully authenticated'"; then
  say "GitHub accepts the key at $BRAIN_SSH_KEY ($(remote "ssh -i $BRAIN_SSH_KEY -o IdentitiesOnly=yes -o BatchMode=yes -T git@github.com 2>&1 | sed -n 's/^Hi \([^!]*\)!.*/\1/p'"))"
else
  warn "GitHub does not know the key at $BRAIN_SSH_KEY yet. Add it to the teenydroid account at https://github.com/settings/keys:
       $(remote "cat $BRAIN_SSH_KEY.pub")"
fi
if remote "test -d $BRAIN_DIR/.git"; then
  say "Brain checkout present at $BRAIN_DIR ($(remote "git -C $BRAIN_DIR rev-parse --short HEAD"))"
else
  say "Cloning the brain into $BRAIN_DIR"
  remote "GIT_SSH_COMMAND='ssh -i $BRAIN_SSH_KEY -o IdentitiesOnly=yes -o BatchMode=yes' git clone -q $BRAIN_REMOTE $BRAIN_DIR" \
    || die "could not clone $BRAIN_REMOTE into $BRAIN_DIR with $BRAIN_SSH_KEY"
fi
# A registry image is pulled so the box runs what is published; an image that
# only exists on the box (built there by hand while nobody publishes one) is
# used as it is. Neither is a die unless there is nothing to run at all.
say "Pulling $OPENCOMPANY_IMAGE"
if ! remote "docker pull -q $OPENCOMPANY_IMAGE >/dev/null 2>&1"; then
  remote "docker image inspect $OPENCOMPANY_IMAGE >/dev/null 2>&1" \
    || die "could not pull $OPENCOMPANY_IMAGE and the box has no such image; set OPENCOMPANY_IMAGE in deploy/.env to one it can reach"
  warn "$OPENCOMPANY_IMAGE could not be pulled; using the copy already on the box"
fi
# The definition is data the agents rewrite. Validate what is on the box
# before it is loaded, so a broken self-edit is this line rather than a
# crash-looping container.
remote "docker run --rm -v $BRAIN_DIR:/data:ro --entrypoint opencompany $OPENCOMPANY_IMAGE check /data/company/teeny" \
  || die "the brain's definition at $BRAIN_DIR/company/teeny does not validate; fix it (or revert it in git) before deploying"

# --- build -----------------------------------------------------------------
say "Building the image on the box ($GIT_SHA)"
remote "$COMPOSE build" || die "image build failed"

# Keep the outgoing image so a bad deploy can be undone in one command.
remote "docker image inspect $PROJECT:current >/dev/null 2>&1 && docker tag $PROJECT:current $PROJECT:previous || true"
remote "docker tag $PROJECT:current $PROJECT:$GIT_SHA || true"
remote "docker image inspect $PROJECT-web:current >/dev/null 2>&1 && docker tag $PROJECT-web:current $PROJECT-web:previous || true"
remote "docker tag $PROJECT-web:current $PROJECT-web:$GIT_SHA || true"

if [ "$BUILD_ONLY" -eq 1 ]; then
  # cutover.sh needs the image on the box before it stops the old agent, so
  # that the account is quiet for seconds rather than for a build.
  say "Built $GIT_SHA. Nothing was started (--build-only)."
  exit 0
fi

# --- release ---------------------------------------------------------------
say "Starting the stack"
remote "$COMPOSE up -d --remove-orphans" || die "compose up failed"

say "Waiting for the agent to answer"
for attempt in $(seq 1 60); do
  if remote "curl -fsS --max-time 3 http://127.0.0.1:$BOT_PORT/health >/dev/null"; then
    say "Healthy after ${attempt}s"
    remote "curl -sS http://127.0.0.1:$BOT_PORT/queue" || true; echo
    # The site is not worth failing the agent's deploy over, so it is checked
    # and reported rather than gated on.
    if remote "curl -fsS --max-time 5 -o /dev/null http://127.0.0.1:$WEB_PORT/"; then
      say "The landing page answers on 127.0.0.1:$WEB_PORT"
    else
      warn "the landing page is not answering on 127.0.0.1:$WEB_PORT; check: $COMPOSE logs web"
    fi
    # The brain is gated, like the agent: a definition that fails to load is
    # the failure this deploy most needs to notice.
    say "Waiting for the brain to answer"
    brain_up=0
    for brain_attempt in $(seq 1 90); do
      if remote "curl -fsS --max-time 3 -o /dev/null http://127.0.0.1:$BRAIN_PORT/healthz"; then
        say "The brain answers on 127.0.0.1:$BRAIN_PORT after ${brain_attempt}s (ssh -L $BRAIN_PORT:127.0.0.1:$BRAIN_PORT $DEPLOY_HOST)"
        brain_up=1
        break
      fi
      sleep 1
    done
    if [ "$brain_up" -eq 0 ]; then
      printf '\n\033[1;31mThe brain never answered. Recent logs:\033[0m\n' >&2
      remote "$COMPOSE logs --tail 60 opencompany" >&2 || true
      die "the brain did not come up on 127.0.0.1:$BRAIN_PORT; the agent is running, the brain is not"
    fi
    say "Deployed $GIT_SHA. Logs: ./deploy/deploy.sh --logs   Undo: ./deploy/deploy.sh --rollback"
    exit 0
  fi
  sleep 1
done

printf '\n\033[1;31mHealth check never passed. Recent logs:\033[0m\n' >&2
remote "$COMPOSE logs --tail 60 bot" >&2 || true
die "deploy did not come up; run ./deploy/deploy.sh --rollback to restore the previous image"
