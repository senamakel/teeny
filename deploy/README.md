# Deploying teeny

teeny runs on `174.138.35.76` (`robot1`, user `droid`, key `~/.ssh/id_rsa`;
override with `DEPLOY_HOST` / `DEPLOY_KEY`) as a Docker Compose stack. The box
is teeny's alone now, but nothing here takes a public port anyway: host nginx
owns :80 and :443 and proxies the one HTTP surface that faces outward.

```sh
cp .env.example .env                 # the agent's credentials: X, Exa, Giphy
cp deploy/.env.example deploy/.env   # the infrastructure around it
./deploy/deploy.sh                   # check, sync, build, release, health check
```

Every push to `main` also deploys, through `.github/workflows/deploy.yml`:
the checks in `.github/workflows/check.yml` (the Rust workspace's fmt, clippy,
tests and duplicate-crate check; the legacy service's lint, typecheck, build
and tests; the web build; shellcheck and the entrypoint tests -- the same
workflow every pull request runs) pass on the runner, then the same script
runs with `--keep-env`,
which leaves the box's two `.env` files untouched and reads what it needs to
verify from them over SSH. The workflow holds one secret, `DEPLOY_SSH_KEY`, a
dedicated key authorised for `droid@robot1`; the host key is pinned in
`deploy/known_hosts`. Changing configuration is still a deploy from a machine
that has the `.env` files.

```sh
./deploy/deploy.sh --skip-checks   # skip the local cargo fmt/clippy/test pass
./deploy/deploy.sh --keep-env      # do not ship .env files; what CI runs
./deploy/deploy.sh --build-only    # sync and build, start nothing
./deploy/deploy.sh --shadow        # the Rust bot in dry-run beside the live legacy one
./deploy/deploy.sh --logs          # tail the running stack
./deploy/deploy.sh --down          # stop it (volumes are kept)
./deploy/deploy.sh --rollback      # put the previous (Rust) image back
```

The local checks are the Rust ones and need `vendor/openhuman` checked out
recursively (`git submodule update --init --recursive vendor/openhuman`); the
image is built on the box from the rsynced tree, submodules included, so the
script refuses to sync without them. Nothing runs `npm` at the root any more:
the legacy service's checks run in CI, from `legacy/`.

## The stack

| Container          | What it does                              | Host port         |
| ------------------ | ----------------------------------------- | ----------------- |
| `teeny-bot-1`      | the agent (Rust, the root `Dockerfile`): X ingest, the webhook, `/agent/run`, replies, the scheduled posts, Telegram, the browser | `127.0.0.1:3110`  |
| `teeny-legacy-bot-1` | the TypeScript agent it replaces, `legacy` profile only: the shadow run and the rollback | `127.0.0.1:3111`  |
| `teeny-web-1`      | the landing page, teeny.enamakel.com       | `127.0.0.1:3120`  |
| `teeny-teeny-cortexdb-1` | the Rust bot's memory: its own CortexDB, the one TinyMemory store every agent shares ("The shared memory" below) | `127.0.0.1:3143` |
| `cortex`           | the legacy service's memory, **not this stack's**: the shared CortexDB, reached as `cortexdb`; the Rust bot only imports from it | `127.0.0.1:3141`, `172.17.0.1:3141` |
| `teeny-mongo-1`    | the mention queue                          | `127.0.0.1:27020` |
| `ladder`           | the model endpoint, owned by no project    | `127.0.0.1:6969`, `172.17.0.1:6969` |
| `teeny-opencompany-1` | the brain: the OpenCompany host + console over the public brain checkout | `127.0.0.1:3130` |
| `teeny-brain-sync-1`  | commits the brain after every run and pushes it to GitHub | — |

The agent is one container. It used to be three -- ingest, reply and chart
over the same image -- on the argument that retrieval has a deadline (X's
search reaches back only 7 days, and every post retrieved is billed) and
replying does not, so the halves should restart independently. In practice
they were always deployed together, and three logs to read was the cost paid
daily for a flexibility used never. `SERVICE_ROLE` still exists in the code,
so the split is a compose change if it is ever wanted again; the constraint to
remember then is that the chart cron and the Telegram poller must each run in
exactly one place.

## The bot image

The root `Dockerfile`, built on the box by `deploy.sh` with BuildKit (the
cargo registry and target directories are cache mounts, so a rebuild after
a change in `crates/` compiles the changed crates and links):

| Stage          | What it makes |
| -------------- | ------------- |
| `build-base`   | `rust:1.96.1-bookworm` plus the X11/xdo/Xtst/Xi, evdev, ALSA, udev and OpenSSL headers OpenHuman links |
| `teeny`        | `cargo build --release --locked -p teeny` |
| `tinycomputer` | `libtinycomputer.so` and the `modules.toml` naming its SHA-256, by `vendor/openhuman/vendor/tinycomputer/scripts/build-module`, from the same vendored SHA as the core |
| `runtime`      | `debian:bookworm-slim` with `chromium`, the Noto fonts (core and colour emoji), `tini`, `gosu` and the X runtime libraries; the build fails if `ldd` finds a library missing |

In the runtime image:

```text
/usr/local/bin/teeny              the service; `teeny healthcheck` is the HEALTHCHECK
/usr/local/bin/teeny-entrypoint   docker/entrypoint.sh, under tini
/opt/tinycomputer/                the module and its modules.toml: root:root, 755/644,
                                  which is what the TinyBus loader insists on
/app/                             SOUL.md, CONTEXT.md, skills/, narrative/, assets/,
                                  calendar.json -- read-only, the working directory
/data/                            the bot-data volume
```

The volume, laid out by the entrypoint on every boot:

```text
/data/openhuman/        OPENHUMAN_WORKSPACE: config.toml, sessions, cron jobs.db;
                        HOME is /data/openhuman/home (OpenHuman's keyring and skills)
/data/chrome-profile/   the browser's persistent profile: logins and cookies
/data/media/            generated media; charts/, moon/, likeness/, downloads/,
                        reference-sheet.json
/data/articles/         the article store
/data/state/            state.json (the ingest cursor), quotes.json,
                        narrative-state.json, backrooms/
```

The legacy service wrote the same volume flat (it was mounted at `/app/data`).
On first boot the entrypoint **copies** those files into the layout above,
once each, and never overwrites a destination that exists; the originals stay
put so the legacy service can be started again on the same volume. It also
clears the `Singleton*` locks a killed Chromium leaves in the profile, which
would otherwise refuse every launch in a recreated container. The path keys
in `.env` (`CHART_OUTPUT_DIR=data/charts` and the rest) are the legacy ones;
`compose.prod.yaml` overrides them for the bot with the `/data` paths.

The container is locked down: `read_only: true` with a 512 MB tmpfs `/tmp`,
`shm_size: 1gb` for Chromium, `no-new-privileges`, and `cap_drop: ALL` with
four capabilities added back -- `CHOWN`, `DAC_OVERRIDE`, `SETUID`, `SETGID`.
Those are the entrypoint's, which starts as root to create the layout inside a
volume root the legacy uid owns and hand it to uid 10001, then `gosu`s to
10001; teeny and Chromium run with none of them. Poke around as the bot's
user, so nothing on the volume ends up root-owned:

```sh
ssh droid@174.138.35.76 'docker exec -u teeny teeny-bot-1 teeny --help'
```

The legacy service's `NODE_OPTIONS=--dns-result-order=ipv4first` is gone with
Node; the same problem (api.telegram.org's AAAA record, no v6 route) is
handled by disabling IPv6 inside the bot's container (`sysctls`).

## Moving to the Rust bot

Three steps, each reversible: a shadow run, the cutover, and the clean-up.
Until the cutover, `deploy.sh` refuses a plain deploy over the running legacy
bot, so a push to `main` never does it by accident.

### 1. The shadow run

The legacy service stays live on the account; the Rust bot runs beside it
with both gates shut (`DRY_RUN=true`, `TELEGRAM_DRY_RUN=true`), on its own
volume (`bot-shadow-data`) and its own copy of the mention queue
(`teeny_shadow`), so nothing it does can reach the account or hide a mention
from legacy. `compose.shadow.yaml` holds those overrides. It also blanks the
Telegram token and turns the activity stream off, because Telegram allows one
poller per token and X one stream per app, and legacy holds both.

In `deploy/.env`, put legacy on the webhook's port for the duration:

```sh
BOT_PORT=3111          # the Rust bot, drafting
LEGACY_BOT_PORT=3110   # legacy, where nginx sends X's deliveries
```

then:

```sh
./deploy/deploy.sh --shadow
```

It builds both images, stops the legacy `bot` container, starts `legacy-bot`
on the same `bot-data` volume and the Rust bot on the shadow volume, and
gates on both answering. The Rust bot's ingest reads X on its own, so the
shadow run pays for every X read twice: keep it to the days it takes.

The shadow bot remembers under a root of its own in `teeny-cortexdb`
(`TEENY_MEMORY_ROOT=team:teeny-shadow`, from `compose.shadow.yaml`), so its
dry-run captures never reach the root the live bot will use. Give it the
account's memory before judging its drafts: import the legacy store into
that root, once the stack is up, and again whenever its recall looks stale:

```sh
ssh droid@174.138.35.76 'docker exec -u teeny teeny-bot-1 teeny memory import --dry-run'
ssh droid@174.138.35.76 'docker exec -u teeny teeny-bot-1 teeny memory import'
```

"The shared memory" below has what the import carries and how it resumes.

Diff the drafts against what legacy did with the same input:

```sh
# The journal each one keeps: legacy's posts and replies, the Rust bot's drafts.
ssh droid@174.138.35.76 'curl -sS 127.0.0.1:3110/mind' > legacy-mind.json
ssh droid@174.138.35.76 'curl -sS 127.0.0.1:3111/mind' > rust-mind.json
# The drafts as they were logged, slot by slot and mention by mention.
ssh droid@174.138.35.76 'docker logs --since 24h teeny-bot-1 2>&1' | grep -i draft
ssh droid@174.138.35.76 'docker logs --since 24h teeny-legacy-bot-1 2>&1' | grep -iE 'draft|posted|replied'
```

Each scheduled slot should produce a Rust draft at the same time legacy
posts, of the same kind, through the content filter and house style; each
mention legacy answers should have a Rust draft (or the same decision to
skip). `/ledger` on each shows what the inference cost. Re-run
`./deploy/deploy.sh --shadow` to ship fixes during the shadow run.

### 2. The cutover

Take the port swap back out of `deploy/.env` (`BOT_PORT` 3110, no
`LEGACY_BOT_PORT`), set `DRY_RUN=false` in `.env` when the Rust bot is to
publish, then:

```sh
./deploy/cutover.sh        # what it will do
./deploy/cutover.sh --go
```

It keeps the running legacy image as `teeny-legacy:current`, builds the Rust
image, stops the legacy bot, snapshots `teeny_bot-data` to
`teeny_bot-data-legacy`, starts the Rust bot on the volume and the webhook's
port, and proves it healthy and able to reach the ladder. The account is
quiet only between the stop and the health check. Browser logins made during
the shadow run are on `bot-shadow-data`; copy its `chrome-profile/` across
with a throwaway container if they are worth keeping.

Then import the legacy store again, now into the live root (`team:teeny`,
the bot's own environment): the legacy service has stopped writing, so
this copy is the whole of it.

```sh
ssh droid@174.138.35.76 'docker exec -u teeny teeny-bot-1 teeny memory import'
```

To have most of it in place already, run the same import into the live
root a day or so before the cutover, from the shadow container
(`docker exec -u teeny -e TEENY_MEMORY_ROOT=team:teeny teeny-bot-1 teeny
memory import`). The run at the cutover then stores only what legacy wrote
since; it still reads the whole scope, because its cursor file is on the
live volume and the earlier one was on the shadow's, and everything already
stored is a replay. Until it finishes, recall in the live bot comes back
thin, not wrong.

### Rolling back

```sh
./deploy/cutover.sh --undo     # back to the TypeScript service
./deploy/deploy.sh --rollback  # back to the previous Rust build
```

`--undo` stops the Rust bot, hands `media/` and `articles/` (the directories
both layouts share) back to the legacy uid, and starts `legacy-bot` on the
webhook's port over the flat files it left behind. Its cursor is where it was
at the cutover, so its next poll re-reads (and is billed for) what the Rust
bot already saw; the mention queue's replied set keeps it from answering
anything twice. `deploy.sh --rollback` refuses to put the legacy image back
through the Rust bot's service definition, which cannot run it.

### 3. The clean-up

Once the Rust bot has posted and replied for a few days: remove
`teeny-legacy-bot-1`, the `teeny_bot-data-legacy` snapshot and
`bot-shadow-data`, then delete `legacy/` and the `legacy-bot` service in a
follow-up PR.

## The two shared containers

`ladder` and `cortex` are not this stack's, and they are not on its network.
Each publishes on the host's docker0 address as well as loopback, and
`compose.prod.yaml` pins the names `ladder` and `cortexdb` to that address
with `extra_hosts` (`host-gateway`, 172.17.0.1). The mapping is in the
container's `/etc/hosts`, so it is fixed at `up` and does not care whether
the container behind it is restarted, recreated or replaced. Whoever starts
either one has to keep the docker0 bind: `-p 172.17.0.1:6969:6969` for the
ladder, `-p 172.17.0.1:3141:3141` for cortex.

They used to be reached by `docker network connect teeny_default <name>`
instead -- once, by hand or by `deploy.sh`. That is runtime state on the old
container: `ladder-up --pull` recreated the ladder on 2026-09-15 and the new
one was on no network but `bridge`, so `ladder` stopped resolving from the
bot. Every morning post and the chart failed on `getaddrinfo EAI_AGAIN
ladder` for three days while every container stayed healthy and the page
stayed up. `deploy.sh` now proves the ladder answers from a container wired
like the bot before it releases, and refuses to deploy when it does not.

## The morning post

`MORNING_ENABLED=true` with `IMAGE_GEN_ENABLED=true` and `VIDEO_GEN_ENABLED=true`
in the box's `.env` (set on 2026-09-14) schedules the good-morning post at
`MORNING_CRON` (07:00 UTC since 2026-09-14, the first of the day's three slots). It draws and films through the box's own ladder,
so that ladder has to be on a router that counts Surplus's prepaid credit as
balance (llm-ladder-router #21) and have credit to spend; `ladder-up --pull`
after a router release.

## The day's three posts

Since 2026-10-04 the calendar is three own posts a day, eight hours apart,
on a three-day rota (`POST_ROTA_DAYS=3`, anchored on 2026-09-14): the
morning post at 07:00 every day (`MORNING_ENABLED`); at 15:00 the
afternoon slot, one cron (`AFTERNOON_CRON`), with the candlestick chart on
day 0, the moon chart on day 1 and the lore post on day 2 (`CHART_ENABLED`,
`MOON_ENABLED`, `LORE_ENABLED`); at 23:00 the evening slot, one cron
(`EVENING_CRON`), with the article on day 1 (`ARTICLE_ENABLED`, and
`ARTICLE_PUBLISH` for it to reach X), the serial on day 2 when
`NARRATIVE_ENABLED`, and the quote post otherwise (`QUOTE_ENABLED`). The
box's `.env` must not pin the old four-slot values (`POST_ROTA_DAYS=7`,
`CHART_ROTA_SLOT=3`, `POST_MIN_GAP_HOURS=6`); the old per-job crons
(`CHART_CRON`, `MOON_CRON`, `LORE_CRON`, `QUOTE_CRON`) are ignored. The quote post's two searches
are billed like every other X read; `quotes.json` on the `bot-data` volume
(`state/quotes.json` for the Rust bot, carried over from the legacy flat
`quotes.json`) is what stops it quoting a post twice, so migrate that volume
with the rest.

## The brain

Teeny's brain is an [OpenCompany](https://github.com/tinyhumansai/opencompany)
company -- agents, skills, workflows, ledgers, notes -- whose whole state is
the public repository `git@github.com:teenydroid/brain.git`, checked out in
this repo as the `brain/` submodule and, on the box, as a live working tree at
`BRAIN_DIR` (`/home/droid/brain`). That checkout **is** the host's data root:
the definition under `company/teeny/`, the shared skills under `skills/`, and
the runtime record under `companies/teeny/`. The agents revise the definition
themselves, and `brain-sync` commits and pushes whatever changed every
`BRAIN_SYNC_INTERVAL` seconds, so every run is on GitHub within a minute.

Two containers, both running as the box user so the files they write are the
checkout owner's:

- `opencompany` runs the host from `OPENCOMPANY_IMAGE` -- pulled, never built
  here -- with `deploy/opencompany/entrypoint.sh` in place of the image's own.
  Inference goes to the ladder like everything else on the box
  (`OPENCOMPANY_INFERENCE_URL` / `OPENCOMPANY_INFERENCE_KEY`, from the
  environment, never from the checkout). The console is served by the host at
  `/` on loopback only.
- `brain-sync` (`brain/sync/`, built from the submodule pinned here -- never
  from the live checkout the agents can write to) pulls, stages, gates and
  pushes. The
  gates: `gitleaks` over the staged tree, then a grep of the staged diff for
  every value in the two `.env` files, because the ladder key has no shape a
  scanner recognises. A hit unstages and logs `REFUSED`; nothing is
  committed. It pushes as the teenydroid account (`BRAIN_SSH_KEY`).
  `docker exec teeny-brain-sync-1 sync-now` runs a tick at once.

### What is and is not public

`brain/.gitignore` denies everything and re-includes the record by name:
the definition, the skills, and under `companies/teeny/` the event log,
journal, runs, board, ledgers, facts, notes and memory. Never tracked:
`secrets/`, `keys/`, `users.json`, session and login-code hashes, the host's
`config.toml`, `instance-id`, `.lock`, the MCP registry (`mcp/`), the agent
journal (`openhuman/`), the agent sandboxes (`harness/`), `tmp/`, `logs/`.
The allowlist is the rule; the sync gates are the backstop.

### Reaching the console

```sh
ssh -L 3130:127.0.0.1:3130 droid@174.138.35.76   # then http://localhost:3130
```

Sign-in is email mode with `BRAIN_ADMIN_EMAIL` as the standing admin grant
(`OPENCOMPANY_ADMIN_EMAIL`). There is no mail transport, so the first
password is minted offline. It takes the data-root lock, so the host has to
be stopped for the one command:

```sh
ssh droid@174.138.35.76
cd ~/teeny/deploy
docker compose -f compose.prod.yaml stop opencompany
docker compose -f compose.prod.yaml run --rm --no-deps --entrypoint opencompany opencompany \
  issue-password --company teeny --email "$BRAIN_ADMIN_EMAIL" --home /data   # password on stdin
docker compose -f compose.prod.yaml start opencompany
```

### One-time setup on the box

1. The teenydroid account's SSH key. `deploy.sh` generates `BRAIN_SSH_KEY`
   on the box if it is missing and prints the public half until GitHub
   accepts it; add it to the **teenydroid** account at
   `https://github.com/settings/keys`. Nothing else on the box changes: the
   sidecar and `deploy.sh` pass the key explicitly, `github.com` in
   `~/.ssh/config` keeps using `droid`'s own key and account, and only the
   opt-in alias `git@github-teenydroid:` uses the teenydroid key. `gh` on
   the box stays signed in as it was. The private key never leaves the box.
2. `OPENCOMPANY_IMAGE`, `BRAIN_ADMIN_EMAIL`, `BRAIN_UID`/`BRAIN_GID` in
   `deploy/.env`. There is no OpenCompany image published yet as of
   2026-09-14; the value is whatever tinyhumansai publishes, built with at
   least the `openhuman mcp` features.
3. `./deploy/deploy.sh` clones the brain into `BRAIN_DIR` on the first run,
   pulls the image, validates the definition with `opencompany check`, and
   gates the release on `/healthz` answering on `127.0.0.1:3130`.

### How the brain reloads itself

A running host never re-reads `company.toml`. The contract is a file:
anything that wants the edited definition loaded touches
`/data/.reload-requested` and stops. The entrypoint sees it, restarts the
server, and the new definition is live in seconds. The `curator` agent does
this after every self-edit; `brain-sync` does it after pulling a change to
`company/` or `skills/` from GitHub, so editing the definition on the web is
the same as the brain editing itself. No docker socket is mounted anywhere.

### Before changing it

- **`BRAIN_DIR` is live state.** It is excluded from the rsync and never
  reset by the deploy. To move it, stop both containers, move the directory,
  change `BRAIN_DIR`, deploy.
- **The `.gitignore` in the brain is the security boundary.** Re-including a
  path there publishes it. `secrets/` and `keys/` hold plaintext.
- **`opencompany check` runs before every deploy** against what is on the
  box, which is what the agents last wrote -- not what is in the submodule.
  A failing check is fixed in the brain repo (or reverted there), not here.

## The landing page

`web/` is a separate Next.js app with its own image (`teeny-web:current`),
built from the same `deploy.sh` run and published on `127.0.0.1:3120`. Host
nginx puts it on the internet as `teeny.enamakel.com`; the vhost is
`deploy/nginx/teeny.enamakel.com.conf` and `deploy/web-vhost.sh` installs it
and issues the certificate, in the right order, once. Cloudflare proxies the
name and reaches this origin over HTTPS.

The page reads its live reflection feed from `MIND_FEED_URL` (deploy/.env)
and shows labelled samples while that is unset. A bad site build never fails
the agent's deploy: the health step reports the page rather than gating on it.

## The shared memory

The Rust bot remembers in one TinyMemory store that every agent shares
through OpenHuman: `teeny-cortexdb`, a CortexDB of this stack's own, its data
in the `teeny-cortexdb-data` volume (migrate it, never recreate it). The
runtime installs it as OpenHuman's memory engine; every agent is bound to the
root `TEENY_MEMORY_ROOT` (`team:teeny`), so the learnings and beliefs there
are shared, each agent's own turns are logged at its node below it, and what
the account captures itself (mentions, replies, chat, its posts, notes) is
stored at the root with the labels, keys and person it always had.

- **Reaching it.** The bot gets `TEENY_MEMORY_URL=http://teeny-cortexdb:3141`
  and `TEENY_MEMORY_TOKEN=$TEENY_CORTEX_API_KEY` from compose; the key lives
  only in `deploy/.env`, and `deploy.sh` refuses to deploy without it.
  OpenHuman's engine sends that key only over HTTPS or to loopback, so the bot
  relays to the store from `127.0.0.1` inside its own container: the key goes
  over the stack's private network, as the legacy key always has. An
  operator on the box reaches the store on `127.0.0.1:3143`.
- **Health.** `teeny_cortex_health` (hourly, and at boot) reads this store's
  `/v1/admin/ready` now, not the legacy one's.
- **Embeddings.** Pinned on the first ingest like the legacy store's:
  `TEENY_CORTEX_EMBEDDING_MODEL` (default `vectors-oai3`, the Surplus tier
  robot1 can reach) and `TEENY_CORTEX_EMBEDDING_DIMS` (default 3072). Check
  the dimension the tier returns before the first import; changing either
  afterwards orphans everything stored.
- **Off.** Without `TEENY_MEMORY_TOKEN` memory is off: every recall reads as
  unavailable and nothing is captured, and the bot still runs.

### Importing the legacy memory

```sh
teeny memory import [--dry-run] [--since <date|timestamp>] [--cursor <file>]
```

It reads the legacy scope (`CORTEX_API_URL`, `CORTEX_SCOPE`, as
`CORTEX_ACTOR` with `CORTEX_TOKEN`) through the one listing that store always
answered fast: `GET /v1/events` with `view=local` and the label filter
`x,telegram` (every capture the account made carries one), 200 a page, never
a recall, a heavy view or a person filter (see the quirks in "The memory tier
is external" below). Each request has a 30-second deadline and is retried
four times with doubling backoff on a timeout, a 429 or a 5xx; the writes go
two at a time.

Every event is turned back into the capture it came from and stored exactly
as the bot stores a live one: its text (a message) or JSON (an activity), its
labels as tags, its time, its person, and its idempotency key
(`x:<kind>:2:<id>`, or `cortex:<event id>` when the listing does not carry
one). TinyMemory derives an item's identity from that, so a rerun stores
nothing twice, and neither does a mention the bot already captured itself.
`--since` leaves out what was observed before it; `--dry-run` reads and
counts everything and writes nothing.

The cursor file (`/data/state/memory-import-<root>.json` by default) records
the listing's next page after every page, so an import that stops goes on
from there when run again; a finished run records the newest event it read,
and the next run stops when it reaches it. The report counts what was read,
stored, already there and skipped, per kind (`mention`, `reply`, `message`,
and each activity's own kind: `chart_post`, `like`, ...).

## The memory tier is external

The legacy service's memory lives in the shared CortexDB on the box: the container named
`cortex`, published on `127.0.0.1:3141` and `172.17.0.1:3141`, with its data
in the `cortex-data` volume. It is nobody's compose project here and it is
not this stack's to start or stop. The agent reaches it as
`http://cortexdb:3141` through the `extra_hosts` entry that maps `cortexdb`
to the docker0 address ("The two shared containers" above); recreating it
needs nothing on this side as long as it keeps the `172.17.0.1:3141` bind.

`CORTEX_TOKEN` in `.env` and `CORTEX_API_KEY` in `deploy/.env` are the key
that container was started with; `deploy.sh` refuses to deploy while the two
differ.

The scope `agent:tiny-x-bot` was moved there on 2026-09-14 from the older
instance in `/opt/cortexdb` (`cortexdb-cortexdb-1`, port 3142, volume
`teeny_cortexdb-data`) with `POST /v1/export` on the old side and
`POST /v1/import` on the new: 903 events, all accepted, re-embedded on import
because the two instances use different embedding sizes (3072 and 1024). The
export is kept in `~/cortex-migration/` on the box. The old instance stays up
for as long as tinysweeper still points at it; teeny is no longer attached.

### Embeddings on robot1

The store embeds through the ladder, and its embedding model is pinned to the
data directory for life. robot1 has no Venice key, so the fleet's `vectors`
ladder (Venice BGE-M3) is stripped from its router config; from 2026-10-04 the
store embedded nothing and recall quietly became keyword-only. Since
2026-10-05 robot1's store is pinned to `vectors-oai3` instead, a Surplus-only
ladder (workspace#19), through `~/.config/cortex/cortex.local.env`:

```
CORTEX_EMBEDDING_MODEL=vectors-oai3
```

The BGE-M3 vectors were moved aside to `vector.bge-m3.20261005` in the
`cortex-data` volume, not deleted, and the corpus re-embedded with
`POST /v1/admin/index-audit {"repair":true,"vectors":true,"scope":…}`. That
endpoint is cut off at 120 s server-side but keeps what it re-embedded, so a
scope is re-embedded by calling it repeatedly
(`~/.cache/cortex-reembed/loop.sh <scope>`). The bot logs
`memory: the store cannot embed` hourly whenever this breaks again.

The `cortexdb` and `tika` services in `compose.prod.yaml` are behind the
`memory` profile so a plain `up` leaves them alone; `--profile memory` starts
a private CortexDB on a box that has no shared one.

## The ladder

The agent and CortexDB both call the ladder, so there is one egress path and
one set of provider keys on the box rather than a second copy inside a
database. It is a container this project does not own and it binds
`127.0.0.1`, which a container cannot reach through `host.docker.internal`.
Two ways to point at it, both in `deploy/.env`:

- **Same box** (what runs today). Leave `LADDER_URL` empty. `deploy.sh` joins
  the container named `ladder` to `teeny_default`, which is what makes
  `http://ladder:6969` resolve. Compose cannot adopt a container it did not
  create, so this is a script step rather than a compose entry, and it has to
  be repeated whenever the ladder container is recreated — re-running the
  deploy does it. The network is created wearing compose's own labels, because
  compose refuses to use one it cannot recognise as its own.
- **Another box.** Set `LADDER_URL` to something reachable *from a container
  here*. `deploy.sh` proves it answers `/models` before releasing.

**This is the failure that took teeny offline before it had a repo.** The
ladder container was recreated, its attachment to the stack's network went with
it, and from then on every model call failed DNS: no replies drafted, and
CortexDB's consolidation erroring on every pass. Nothing crashed and nothing
looked unhealthy — the ingest half kept polling X and logging clean lines for
days. That is why `deploy.sh` refuses to release without proving the model
endpoint answers, and why the cutover proves it again from inside the running
container.

## The two .env files

`deploy/.env` is the infrastructure: database, ports, memory tier, model
endpoint. The repo-root `.env` is the agent's own credentials and persona
settings. Both are excluded from the rsync and copied explicitly, so a missing
local file can never wipe a working one on the box.

`CORTEX_API_KEY` (deploy/.env) and `CORTEX_TOKEN` (.env) are the same secret
from two sides — the service's own key, and the client's copy of it. They are
checked against each other before every deploy, because a mismatch fails at the
first recall rather than at boot. `TEENY_CORTEX_API_KEY` (deploy/.env) is the
Rust bot's own store's key, and only there: compose hands it to the bot.

`CORTEX_ACTOR` and `CORTEX_SCOPE` are `agent:tiny-x-bot` and must stay that
way: everything Tiny remembers is filed under that scope, and changing it does
not migrate the memory, it hides it.

`DRY_RUN=false` in `.env` is the only thing between a drafted reply and a
published one. The deploy prints which of the two it is shipping.

## The webhook

X delivers to the landing page's name: **`https://teeny.enamakel.com/api/webhooks/x`**,
proxied by Cloudflare to this box, where the vhost in
`deploy/nginx/teeny.enamakel.com.conf` routes that one path to the agent
container (`/webhooks/x` on `127.0.0.1:3110`) and everything else to the
site. Installing or changing the vhost is `./deploy/web-vhost.sh`.

```text
https://teeny.enamakel.com/api/webhooks/x   X Activity API deliveries
https://teeny.tinyhumans.ai/agent/run       ad-hoc tasks, bearer AGENT_API_TOKEN
https://teeny.tinyhumans.ai/safety          the content filter's review page
```

`teeny.tinyhumans.ai` (`deploy/nginx/teeny.conf`) still answers `/webhooks/x`
too, so the old registration would work if it were ever put back; it is not
where X delivers. `/health` and `/queue` are operational, not public, on
either name -- read them over SSH.

Two things about these vhosts, both of which have bitten this box before:

- The 443 block is not optional. Cloudflare connects to the origin over
  HTTPS, and a name with no TLS server of its own falls through to the first
  one nginx has — another project's — which then gets served under this
  domain.
- Test it with SNI. `curl -H 'Host: ...' https://127.0.0.1/` sends no SNI, so
  nginx answers from the default server and the result tells you nothing
  about this vhost. Use `--resolve teeny.enamakel.com:443:127.0.0.1`.

The registration moved from `stonks.rocks` to `teeny.tinyhumans.ai` on
2026-09-08 and from there to `teeny.enamakel.com/api` on 2026-09-14, both
with `npm run webhook`, and X's CRC challenge against the new URL passed
each time.

The second move also changed what delivers. The Account Activity API the
account had been subscribed through is deprecated, and on 2026-09-14 it
refused to associate any freshly created webhook at all (`Webhook with
specified ID not found, or not associated with app ID`, for the new URL and
the old one alike). The subscription is now through the **X Activity API**:
one subscription each for `post.mention.create`, `post.reply.create`,
`post.quote.create`, `post.repost.create`, `like.create` and `follow.follow`,
bound to the webhook by id, which `npm run webhook` creates and lists. Each
delivered post event costs one post credit; the likes and follows are free
and feed the people record that decides who is answered first. The
delivery is one event per request, `{ data: { event_type, payload, ... } }`
with a v2 post as the payload, and the app reads both that and the old
`tweet_create_events` shape.

```sh
cd legacy
npm run webhook                                                 # where X delivers, and who is subscribed
npm run webhook -- --set https://teeny.enamakel.com/api/webhooks/x --confirm
```

The app holds exactly one webhook, so a move is a delete followed by a create
and there is no moment where both are registered — the cron poll covers the
seconds in between. See the header of `legacy/src/scripts/webhook.ts` for the rest,
including why a wrong credential shows up as a `503` rather than a 401.

### Forwarding it

teeny is the only place X can deliver to, so any other service that wants
the account's activity gets a copy from teeny. Set `WEBHOOK_FORWARD_URL` in
`.env` (and `WEBHOOK_FORWARD_TOKEN`, sent as a bearer, if the receiver wants
one) and every delivery whose signature checks out is POSTed there byte for
byte, with X's `x-twitter-webhooks-signature` header still on it, so the
receiver verifies it against the same `X_API_SECRET` and trusts nothing about
the hop. The copy goes out after X has been answered and is not waited for:
a receiver that is slow or down never costs a mention here, and a copy that
does not arrive is logged and lost -- there is no replay, so the receiver
should not be the only reader of anything it cannot afford to miss.

### The stream

Since 2026-08 X delivers only `like.create` and `follow.follow` to the
webhook and silently drops every `post.*` event, for this app and others
([devcommunity 273644](https://devcommunity.x.com/t/273644), acknowledged
by X staff, unresolved as of 2026-09-17). Confirmed here on 2026-09-21: 20
minutes of captured deliveries were all likes while three mentions went by,
and 64 of the 70 mentions queued in the preceding 48 hours had been found by
the cron, minutes late. The same subscriptions deliver every event, posts
included, over `GET /2/activity/stream` within seconds, so the agent holds
that stream open too (`ACTIVITY_STREAM_ENABLED`, on by default, needs
`X_BEARER_TOKEN`). One connection per app; it runs in the ingest role. A
mention that arrives shows up as `queued mentions from activity` with
`source: "stream"` or `"webhook"` in the bot's log, whichever got there
first; the other copy is dropped by event id. Note that X holds the
response headers for the first keep-alive, so "activity stream connected"
appears about 20 seconds after start, and answers 409 until at least one
subscription exists.

The same day the app was replaced (new consumer key, new project
`2095056638054907912`, access token obtained with the OAuth 1.0a PIN flow)
and the webhook re-registered under it as `2102011178583224320`. That
fresh registration delivers `post.*` events again: the first mention after
it landed by webhook, 5 seconds after it was posted. So the fault was tied
to the old app's registration, not to webhooks in general, and the stream
is insurance against it coming back rather than the only working path.

The cron poll is the safety net under all of it: a webhook delivery that fails
is retried for a while and then dropped, a stream reconnect leaves a gap, and
an account that misses a mention entirely has no way to notice. With the
stream on it is a backstop, not the main reader, and `INGEST_CRON` in `.env`
can run hourly rather than every few minutes.

Read the operational endpoints over SSH:

```sh
ssh droid@174.138.35.76 'curl -sS http://127.0.0.1:3110/queue'
```

## How teeny got here

teeny ran on this box for weeks before it had a repository, as `stonks-bot-1`
and `stonks-bot-reply-1` inside the stonks.rocks stack. `deploy/cutover.sh`
performed the move, once, on 2026-09-08:

- the old halves were stopped **before** the new ones started — both reply as
  @TinyMascot off one queue, so running them together means two answers to one
  mention, from one account, in public;
- `stonks_cortexdb-data` (1415 events, 1574 vectors), `stonks_bot-data` (the
  ingest cursor) and the 34-document `mentions` collection were copied, not
  moved, into teeny's own volumes;
- the webhook upstream was repointed from :3101 to :3110, so nothing had to be
  re-registered with X;
- the rest of the box — the stonks website and the whole of prmpt — was
  stopped at the same time, on request. Stopped, not removed: every container
  and every volume is still there.

That version of `deploy/cutover.sh` is in git history
(`git log --follow -p -- deploy/cutover.sh`); the script now performs the
move to the Rust bot ("Moving to the Rust bot" above).

Once teeny has been answering for a while, the leftovers can go:

```sh
ssh droid@174.138.35.76 'docker rm stonks-bot-1 stonks-bot-reply-1 stonks-cortexdb-1 stonks-tika-1'
```

and the `bot` profile should be deleted from the stonks compose file, so a
stonks deploy cannot bring a second agent back up on this account.

## The wallet

There is no wallet in the running service. The agent had one for a day and the
tool was removed; the code and its configuration remain, unwired. `AIRDROP_*` in
`.env` is therefore inert, and `AIRDROP_ENABLED=false` is the state to leave it
in.

The key in `AIRDROP_PRIVATE_KEY` still controls a funded address
(`0x4C3799279dDF44d97a46dd14927789f23bf47589`, roughly 302k $TINY and 0.2 ETH),
so it is still worth treating as a secret: it is not in git, not in the image,
and not in any log line, and the tokens are reachable by anybody holding it.
Move them out and rotate the key if the wallet is not coming back.

## Before the next real change

1. **X credentials.** `X_ACCESS_TOKEN` and `X_ACCESS_TOKEN_SECRET` come from
   `npm run auth:x`, not the developer portal, and must be authorized while
   logged in as the bot account.
2. **Embedding dims are permanent.** `CORTEX_EMBEDDING_MODEL` (`vectors`) and
   `CORTEX_EMBEDDING_DIMS` (3072) were pinned on CortexDB's first ingest.
   Changing either now orphans everything already stored.
3. **The mention queue is not derived data.** Unlike the stonks database, it
   cannot be rebuilt: X's search only reaches back 7 days. Migrate the
   `teeny_mongo-data` volume, never recreate it.
4. **Mongo is pinned to 8.0**, which is what this box (kernel 6.8) has always
   run. Do not copy the newer droplet's pin of 7.0 across: that pin exists
   because 8.x either refuses to start or segfaults in a loop on kernel 7.0.0.
