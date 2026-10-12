#!/bin/sh
# The bot container's entrypoint: lay out the volume, carry the legacy data
# across, clear Chrome's stale locks, then exec teeny as uid 10001.
#
#   tini -- teeny-entrypoint serve        # what the image runs
#   tini -- teeny-entrypoint healthcheck  # any subcommand passes through
#
# The volume (`bot-data`, mounted at TEENY_DATA=/data) is laid out as:
#
#   openhuman/       the OpenHuman workspace: config.toml, sessions, cron
#                    jobs.db, the keyring; HOME is openhuman/home
#   chrome-profile/  the browser's persistent profile (logins, cookies)
#   media/           generated images, video, charts, the reference sheet
#   articles/        the article store
#   state/           cursors and logs: state.json, quotes.json, ...;
#                    memory/LEARNINGS.md, the learnings brief, and its history/
#
# The TypeScript service mounted the same volume at /app/data and wrote it
# flat. Its files are COPIED into the layout above, once each, never moved:
# the originals stay where legacy reads them, so `cutover.sh --undo` puts the
# old service back on its own state. media/ and articles/ already sit where
# the layout wants them. A destination that exists is never overwritten, so
# every boot after the first copies nothing.
#
# The image runs on a read-only root with `cap_drop: ALL`. Started as root,
# this script needs CHOWN, DAC_OVERRIDE, SETUID and SETGID (compose adds
# exactly those): DAC_OVERRIDE to create the layout inside a volume root the
# legacy uid owns, CHOWN to hand it to 10001, SETUID/SETGID for gosu. teeny
# itself runs with none of them. Started as a non-root user (a pinned
# `user:`, or the tests), it skips the ownership heal and runs as itself.
set -eu

DATA="${TEENY_DATA:-/data}"
TEENY_UID="${TEENY_UID:-10001}"
TEENY_GID="${TEENY_GID:-10001}"
# The tests point this at a stub; the image has teeny on PATH.
TEENY_BIN="${TEENY_BIN:-teeny}"

log() { printf '[entrypoint] %s\n' "$*" >&2; }

make_layout() {
  for dir in openhuman openhuman/home chrome-profile media media/downloads articles state; do
    mkdir -p "$DATA/$dir"
  done
}

# copy_once <legacy path> <layout path>, both relative to DATA. Copies through
# a temporary name so an interrupted boot never leaves a half-copied
# destination that the next boot would mistake for a finished one. Plain
# `cp -R`, not `-p`: as root without FOWNER, preserving the legacy owner
# leaves a file root may no longer chmod, and own_layout hands the copy to
# teeny's uid anyway. Symlinks are copied as links, not followed.
copy_once() {
  from="$DATA/$1"
  to="$DATA/$2"
  if [ ! -e "$from" ] || [ -e "$to" ]; then
    return 0
  fi
  partial="$to.migrating"
  rm -rf "$partial"
  cp -R "$from" "$partial"
  mv "$partial" "$to"
  log "migrated $1 -> $2"
}

migrate_legacy() {
  copy_once state.json state/state.json
  copy_once quotes.json state/quotes.json
  copy_once narrative-state.json state/narrative-state.json
  copy_once backrooms state/backrooms
  copy_once memory state/memory
  copy_once charts media/charts
  copy_once moon media/moon
  copy_once likeness media/likeness
  copy_once reference-sheet.json media/reference-sheet.json
}

# Hands the layout to teeny's uid. Only what is not already its own is
# touched, so a large Chrome profile costs a walk, not a rewrite. The volume
# root itself is left alone: legacy owns it, and during a rollback legacy
# has to be able to write there again. `-h`: SingletonLock is a symlink.
own_layout() {
  for dir in openhuman chrome-profile media articles state; do
    find "$DATA/$dir" \( ! -user "$TEENY_UID" -o ! -group "$TEENY_GID" \) \
      -exec chown -h "$TEENY_UID:$TEENY_GID" {} +
  done
}

chromium_running() {
  for comm in /proc/[0-9]*/comm; do
    [ -r "$comm" ] || continue
    case "$(cat "$comm" 2>/dev/null || true)" in
      chromium* | chrome*) return 0 ;;
    esac
  done
  return 1
}

# Chrome refuses a profile whose SingletonLock names another host, and a
# recreated container is another host (its hostname is the container id), so
# the lock a killed browser left behind blocks every later launch. Nothing
# in this container can be using it yet: the browser starts after teeny.
clear_chrome_locks() {
  if chromium_running; then
    log "chromium is running; leaving its profile locks alone"
    return 0
  fi
  find "$DATA/chrome-profile" -maxdepth 2 -name 'Singleton*' -exec rm -f {} +
}

if [ "$(id -u)" -ne 0 ]; then
  make_layout
  migrate_legacy
  clear_chrome_locks
  exec "$TEENY_BIN" "$@"
fi

# Prove gosu can drop privileges before touching anything, so a container
# started without the capabilities says so rather than failing somewhere in
# the migration: the remedy is a compose change.
if ! gosu "$TEENY_UID:$TEENY_GID" true 2>/dev/null; then
  log "FATAL cannot drop privileges to $TEENY_UID:$TEENY_GID with gosu"
  log "FATAL with cap_drop: ALL the container needs cap_add: [CHOWN, DAC_OVERRIDE, SETUID, SETGID]"
  exit 1
fi
make_layout
migrate_legacy
own_layout
clear_chrome_locks
exec gosu "$TEENY_UID:$TEENY_GID" "$TEENY_BIN" "$@"
