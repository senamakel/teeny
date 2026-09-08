#!/usr/bin/env bash
# Move teeny out of the stonks stack and into its own, once.
#
#   ./deploy/cutover.sh              # say what would happen, change nothing
#   ./deploy/cutover.sh --go         # do it
#   ./deploy/cutover.sh --keep-others  # ...but leave prmpt and the stonks site up
#   ./deploy/cutover.sh --undo       # put everything back the way it was
#
# teeny has been running on this box since before it had a repo: the agent is
# `stonks-bot-1` and `stonks-bot-reply-1` inside the stonks.rocks stack, posting
# as the same X account this repo deploys. So this is a migration, not a first
# deploy, and two things follow from that.
#
# The old agent has to STOP BEFORE the new one starts. Both halves reply as
# @TinyMascot off one shared queue; running them together means two answers to
# one mention, from one account, in public.
#
# And the state has to come with it. Three pieces, none of them reproducible:
#
#   stonks_cortexdb-data   everything Tiny remembers about everyone it has
#                          spoken to. Cannot be rebuilt from anything.
#   stonks_bot-data        state.json, the ingest cursor. Lose it and the next
#                          poll re-reads (and re-bills) up to 7 days of posts.
#   stonks.mentions        the mention queue and, in repliedPostIds, the
#                          record of what has already been answered.
#
# Volumes are COPIED, never moved, and the old containers are stopped rather
# than removed, which is what makes --undo possible.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

DEPLOY_HOST="${DEPLOY_HOST:-droid@174.138.35.76}"
DEPLOY_KEY="${DEPLOY_KEY:-$HOME/.ssh/id_rsa}"
REMOTE_DIR="${REMOTE_DIR:-/home/droid/teeny}"
OLD_DIR="${OLD_DIR:-/home/droid/stonks.rocks}"
OLD_PROJECT=stonks
NGINX_CONF="${NGINX_CONF:-/etc/nginx/sites-available/stonks.rocks.conf}"

GO=0
UNDO=0
KEEP_OTHERS=0
# Everything else this box was running. The website halves of stonks and the
# whole of prmpt: stopped, never removed, and their volumes never touched, so
# any of it comes back with a `docker start`.
OTHERS="stonks-web-1 stonks-quotes-1 stonks-mongo-1 prmpt-frontend-1 prmpt-backend-1 prmpt-settler-1 prmpt-mongo-1 prmpt-redis-1"
for arg in "$@"; do
  case "$arg" in
    --go) GO=1 ;;
    --keep-others) KEEP_OTHERS=1 ;;
    --undo) UNDO=1 ;;
    -h|--help) sed -n '2,7p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) echo "unknown argument: $arg" >&2; exit 2 ;;
  esac
done

say() { printf '\n\033[1;35m==>\033[0m %s\n' "$*"; }
die() { printf '\n\033[1;31mfailed:\033[0m %s\n' "$*" >&2; exit 1; }
remote() { ssh -o BatchMode=yes -i "$DEPLOY_KEY" "$DEPLOY_HOST" "$@"; }
envval() { sed -n "s/^$1=//p" "$2" 2>/dev/null | tail -1; }

BOT_PORT="$(envval BOT_PORT deploy/.env)"; BOT_PORT="${BOT_PORT:-3110}"
OLD_BOT_PORT="${OLD_BOT_PORT:-3101}"
COMPOSE="docker compose --project-directory $REMOTE_DIR/deploy -f $REMOTE_DIR/deploy/compose.prod.yaml --env-file $REMOTE_DIR/deploy/.env"
OLD_COMPOSE="docker compose --project-directory $OLD_DIR/deploy -f $OLD_DIR/deploy/compose.prod.yaml --env-file $OLD_DIR/deploy/.env"

if [ "$UNDO" -eq 1 ]; then
  say "Stopping the teeny stack"
  remote "$COMPOSE down" || true
  say "Restarting the stonks halves"
  remote "docker update --restart=unless-stopped stonks-bot-1 stonks-bot-reply-1 stonks-cortexdb-1 stonks-tika-1 >/dev/null" || true
  remote "$OLD_COMPOSE --profile bot up -d bot bot-reply cortexdb tika" || die "could not restart the stonks halves"
  say "Pointing the webhook back at $OLD_BOT_PORT"
  remote "sudo -n sed -i 's|127\\.0\\.0\\.1:$BOT_PORT|127.0.0.1:$OLD_BOT_PORT|' $NGINX_CONF && sudo -n nginx -t && sudo -n systemctl reload nginx"
  say "Restarting everything else this box used to run"
  remote "docker update --restart=unless-stopped $OTHERS >/dev/null 2>&1; docker start $OTHERS >/dev/null 2>&1" || true
  say "Back on the stonks stack. The teeny volumes are still there, untouched."
  exit 0
fi

if [ "$GO" -eq 0 ]; then
  cat <<PLAN

This is a dry run. Nothing has been changed. With --go it would:

  1. build the teeny image on the box (deploy.sh --build-only), so the account
     is quiet for seconds rather than for the length of a build;
  2. stop stonks-bot-1 and stonks-bot-reply-1 and set them restart=no
     --- from here the account answers nothing until step 5;
  3. copy stonks_cortexdb-data -> teeny_cortexdb-data (the memory), and
     stonks_bot-data -> teeny_bot-data and teeny_reply-data (the cursor);
  4. start teeny's mongo and copy the mentions collection into it;
  5. start the teeny stack: bot, bot-reply, cortexdb, tika;
  6. repoint the webhook in nginx from :$OLD_BOT_PORT to :$BOT_PORT, so the URL
     already registered with X keeps working and nothing has to be re-subscribed;
  7. stop stonks-cortexdb-1 and stonks-tika-1, which teeny now has its own of;
  8. stop everything else on this box --- the stonks website and the whole of
     prmpt --- leaving the machine to teeny alone. Skip with --keep-others.
     stonks.rocks and prmpt.cash stop answering at this point.

Nothing is deleted: the stonks containers are stopped, not removed, and the
volumes are copied rather than moved. ./deploy/cutover.sh --undo reverses it.

  ./deploy/cutover.sh --go

PLAN
  exit 0
fi

# --- 1. the image ----------------------------------------------------------
say "Building the teeny image on the box"
./deploy/deploy.sh --build-only || die "build failed; nothing has been changed"

# --- 2. silence the old agent ----------------------------------------------
say "Stopping the stonks agent halves (the account goes quiet from here)"
remote "docker update --restart=no stonks-bot-1 stonks-bot-reply-1 >/dev/null && docker stop stonks-bot-1 stonks-bot-reply-1" \
  || die "could not stop the old agent; refusing to start a second one on the same account"

# --- 3. the volumes --------------------------------------------------------
copy_volume() {
  local from="$1" to="$2"
  remote "docker volume inspect $from >/dev/null 2>&1" || die "no volume $from on the box"
  remote "docker volume create $to >/dev/null && docker run --rm -v $from:/from:ro -v $to:/to alpine sh -c 'cd /from && cp -a . /to/'" \
    || die "copying $from -> $to failed"
  say "  $from -> $to"
}
say "Copying the state that cannot be rebuilt"
copy_volume "${OLD_PROJECT}_cortexdb-data" teeny_cortexdb-data
copy_volume "${OLD_PROJECT}_bot-data" teeny_bot-data
copy_volume "${OLD_PROJECT}_bot-data" teeny_reply-data

# --- 4. the mention queue --------------------------------------------------
say "Copying the mention queue"
OLD_MONGO_USER="$(remote "sed -n 's/^MONGO_ROOT_USERNAME=//p' $OLD_DIR/deploy/.env | tail -1")"
OLD_MONGO_PASS="$(remote "sed -n 's/^MONGO_ROOT_PASSWORD=//p' $OLD_DIR/deploy/.env | tail -1")"
OLD_MONGO_DB="$(remote "sed -n 's/^MONGO_DATABASE=//p' $OLD_DIR/deploy/.env | tail -1")"
NEW_MONGO_USER="$(envval MONGO_ROOT_USERNAME deploy/.env)"
NEW_MONGO_PASS="$(envval MONGO_ROOT_PASSWORD deploy/.env)"
NEW_MONGO_DB="$(envval MONGO_DATABASE deploy/.env)"

remote "$COMPOSE up -d mongo" || die "teeny's mongo did not start"
for _ in $(seq 1 60); do
  remote "docker inspect --format '{{.State.Health.Status}}' teeny-mongo-1 2>/dev/null | grep -q healthy" && break
  sleep 2
done
remote "docker inspect --format '{{.State.Health.Status}}' teeny-mongo-1 | grep -q healthy" || die "teeny's mongo never became healthy"

# Through a file rather than a pipe: an archive streamed over two nested ssh
# hops is one dropped byte away from a half-restored queue.
remote "docker exec ${OLD_PROJECT}-mongo-1 mongodump --quiet -u '$OLD_MONGO_USER' -p '$OLD_MONGO_PASS' --authenticationDatabase admin --db '$OLD_MONGO_DB' --collection mentions --archive=/tmp/mentions.archive" \
  || die "mongodump failed"
remote "docker cp ${OLD_PROJECT}-mongo-1:/tmp/mentions.archive /tmp/mentions.archive && docker cp /tmp/mentions.archive teeny-mongo-1:/tmp/mentions.archive"
remote "docker exec teeny-mongo-1 mongorestore --quiet -u '$NEW_MONGO_USER' -p '$NEW_MONGO_PASS' --authenticationDatabase admin --archive=/tmp/mentions.archive --nsFrom '$OLD_MONGO_DB.mentions' --nsTo '$NEW_MONGO_DB.mentions' --drop" \
  || die "mongorestore failed"
COUNT="$(remote "docker exec teeny-mongo-1 mongosh --quiet -u '$NEW_MONGO_USER' -p '$NEW_MONGO_PASS' --authenticationDatabase admin --eval 'db.getSiblingDB(\"$NEW_MONGO_DB\").mentions.countDocuments({})'")"
say "  $COUNT mentions carried over"
[ "${COUNT:-0}" -gt 0 ] || die "the queue arrived empty; not starting the agent on a blank history"

# --- 5. start teeny --------------------------------------------------------
say "Starting the teeny stack"
remote "$COMPOSE up -d --remove-orphans" || die "compose up failed; run ./deploy/cutover.sh --undo"

for attempt in $(seq 1 90); do
  remote "curl -fsS --max-time 3 http://127.0.0.1:$BOT_PORT/health >/dev/null" && break
  sleep 1
done
remote "curl -fsS --max-time 3 http://127.0.0.1:$BOT_PORT/health >/dev/null" \
  || { remote "$COMPOSE logs --tail 60 bot" >&2; die "teeny never answered; run ./deploy/cutover.sh --undo"; }
say "teeny is answering on 127.0.0.1:$BOT_PORT"

# The agent is useless without the ladder and looks fine without it, so prove
# the path rather than assuming the network attachment survived.
remote "docker exec teeny-bot-1 node -e 'fetch(\"http://ladder:6969/v1/models\",{headers:{authorization:process.env.LLM_API_KEY?\"Bearer \"+process.env.LLM_API_KEY:\"\"}}).then(r=>process.exit(r.ok?0:1)).catch(()=>process.exit(1))'" \
  || die "teeny cannot reach the ladder; run ./deploy/cutover.sh --undo"
say "teeny can reach the ladder"

# --- 6. the webhook --------------------------------------------------------
# X posts to a URL registered in its developer console. Repointing the upstream
# here keeps that URL working, so nothing has to be re-subscribed.
say "Repointing the webhook from :$OLD_BOT_PORT to :$BOT_PORT"
remote "sudo -n sed -i 's|127\\.0\\.0\\.1:$OLD_BOT_PORT|127.0.0.1:$BOT_PORT|' $NGINX_CONF && sudo -n nginx -t && sudo -n systemctl reload nginx" \
  || die "nginx would not reload; the webhook still points at the stopped container"

# --- 7. the leftovers ------------------------------------------------------
say "Stopping the stonks memory tier, which teeny now has its own of"
remote "docker update --restart=no stonks-cortexdb-1 stonks-tika-1 >/dev/null && docker stop stonks-cortexdb-1 stonks-tika-1" || true

# --- 8. the rest of the box ------------------------------------------------
if [ "$KEEP_OTHERS" -eq 0 ]; then
  say "Stopping everything else on this box"
  remote "docker update --restart=no $OTHERS >/dev/null 2>&1; docker stop $OTHERS >/dev/null 2>&1" || true
  remote "docker ps --format '  {{.Names}}'" || true
fi

say "Cut over."
remote "curl -sS http://127.0.0.1:$BOT_PORT/queue"; echo
cat <<DONE

The stonks stack still runs the website; only its two agent halves and its
memory tier are stopped, and its volumes are untouched. Once teeny has replied
to something and the memory reads back, the leftovers can be removed:

  ssh $DEPLOY_HOST 'docker rm stonks-bot-1 stonks-bot-reply-1 stonks-cortexdb-1 stonks-tika-1'

and the bot profile deleted from the stonks compose file, so a stonks deploy
cannot bring a second agent back up on this account.

  ./deploy/cutover.sh --undo   # go back
DONE
