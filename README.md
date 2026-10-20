# teeny

Teeny ([`@teenydroid`](https://x.com/teenydroid)) is the TinyHumans mascot. It is a stock
[OpenHuman](https://github.com/tinyhumansai/openhuman) core plus a brain and a
handful of small programs. There is no custom runtime: OpenHuman runs the
agent loop, cron, memory and tools, and Teeny is the files it reads and the
programs it calls.

| | What | Where |
|---|---|---|
| **Spine** | `teeny-spine`, OpenHuman's own command-line host, built here from the vendored library and run as `serve`. It provides the agent loop, model routing, cron, memory (CortexDB), browser, shell and JSON-RPC. No Teeny logic lives in it. | `spine/`, `vendor/openhuman` |
| **Hands** | Small programs that hold every outside capability, every secret and every hard rule. Models only draft; hands act. | `hands/`, `bin/` |
| **Brain** | Who Teeny is and what it does: identity, known facts, skills, narrative, assets and the schedule. All of OpenHuman's state sits beside it in one SQLite file. | `brain/` |

[`docs/architecture.md`](docs/architecture.md) is the design.
[`docs/hands.md`](docs/hands.md) specifies every hand, and
[`docs/oh-gaps.md`](docs/oh-gaps.md) lists what upstream OpenHuman could add to
make the hands smaller.

## How a post happens

```
cron (in the spine) ──► bin/<job> ──► bin/ask  "draft a post"      the model drafts, in Teeny's voice
                                 ──► hands/guard check              lexicon, links, style, model judge
                                 ──► hands/x post                   posting rules, DRY_RUN, X, the ledger
```

Cron runs **shell** jobs, not agent turns. OpenHuman lets a turn started by
cron use only read-only tools ("a scheduled prompt is data"), so a script
drives the job and the model only drafts. The model never sees a key; the
hands read them from `secrets/teeny.env` themselves.

## Status

- **Spine:** done. It builds, boots from `brain/` (in Docker or on a laptop),
  points itself at the model and CortexDB, and seeds cron from
  `brain/cron.toml`.
- **Hands:** `hands/guard` (the content filter) and `hands/x` (reads, post,
  reply, like, upload, unsay) are built and tested.
- **First live run:** the spine booted, the heartbeat ran from cron,
  `bin/ask` drafted a post in Teeny's voice, `hands/guard` passed it, and
  `hands/x post` returned `dry_run: true`.
- **Next:** the job scripts (morning post, replies), and mention intake
  through the event ledger (built separately). The only scheduled job today is
  `heartbeat`.
- **`DRY_RUN` is on** until `secrets/teeny.env` says `DRY_RUN=0`.

## Run it

### With Docker (how the box runs it)

```sh
git submodule update --init --recursive vendor/openhuman
mkdir -p secrets && chmod 700 secrets
cp deploy/spine.env.example secrets/spine.env    # the model; CortexDB if wanted
cp deploy/teeny.env.example secrets/teeny.env    # the X keys; DRY_RUN=1
chmod 600 secrets/*.env
TEENY_UID=$(id -u) TEENY_GID=$(id -g) docker compose -f deploy/compose.yaml up -d --build
docker compose -f deploy/compose.yaml logs -f spine      # "[spine] ready" once seeded
```

Run anything below inside the container with
`docker compose -f deploy/compose.yaml exec spine <command>`.
[`deploy/README.md`](deploy/README.md) covers the server: updates, backups and
the landing page.

### On a laptop

```sh
git submodule update --init --recursive vendor/openhuman
cargo build --release -p teeny-spine
bin/spine          # mints secrets/core.token and secrets/openhuman.key on first boot
```

## Operate it

```sh
bin/crons                          # every cron job, its next run and last result (a sqlite query)
bin/crons runs heartbeat           # one job's recent runs
bin/ask "who are you?"             # one agent turn, as Teeny
bin/oh cron_list                   # any JSON-RPC method on the running core
bin/seed-cron --dry-run            # what seeding brain/cron.toml would change

hands/x whoami                                        # who the X keys belong to
hands/x search --query '@teenydroid'                  # a recent search, cached
echo '{"text":"gm"}' | hands/guard check              # ok, or a hold with the reason
echo '{"text":"gm"}' | hands/x post                   # guard, rules, then DRY_RUN or X
hands/guard holds list                                # what the guard is holding for review
```

Every hand prints one JSON object. The exit code is `0` when it's done, `2`
when a rule refused (the JSON says which), and `1` on an error. Add `--help` to
any hand or verb for its flags.

To schedule something, add it to `brain/cron.toml` and restart the spine:

```toml
[jobs.morning]
schedule = "0 7 * * *"
tz = "Asia/Kolkata"
command = "bin/morning"
```

## Configure it

Everything secret lives in `secrets/`. That folder is gitignored and outside
the brain.

**`secrets/spine.env`** (what `bin/spine` gives the core at every boot):

| Key | |
|---|---|
| `LADDER_URL` | Any OpenAI-compatible endpoint, run as OpenHuman's `local-openai` runtime, so no TinyHumans account is needed. |
| `LADDER_API_KEY` | Stored in the core's encrypted store on boot. Can be removed afterwards. |
| `SPINE_MODEL` | The model id every workload uses. |
| `SPINE_CONTEXT_WINDOW` | Optional. Otherwise it's read from the endpoint's `/models`. The core trims prompts to this. |
| `CORTEX_API_URL`, `CORTEX_API_KEY` | Optional memory engine. |

**`secrets/teeny.env`** (read by each hand itself; the process environment
can't override it, though `DRY_RUN=1` in the environment can force a rehearsal):

| Key | |
|---|---|
| `DRY_RUN` | On unless set to `0`. |
| `X_BOT_USERNAME`, `X_API_KEY`, `X_API_SECRET`, `X_ACCESS_TOKEN`, `X_ACCESS_TOKEN_SECRET`, `X_BEARER_TOKEN` | X API v2: OAuth 1.0a for writes, the bearer for reads. |
| `POST_MIN_GAP_HOURS`, `REPLY_MIN_INTERVAL_MS`, `REPLY_MAX_INTERVAL_MS`, ... | The posting rules. See `deploy/teeny.env.example`. |
| `MAX_EMOJI_PER_POST`, `SAFETY_JUDGE`, `LINK_ALLOWLIST` | The guard. |
| `GIPHY_API_KEY` | GIFs for posts. |

The spine writes these itself on first boot: `secrets/core.token` (the RPC
bearer) and `secrets/openhuman.key` (the master key for every secret the core
stores). **Back up `brain/storage.db` and `secrets/openhuman.key` together.**
The database is unreadable without its key.

## Layout

```
spine/               teeny-spine, the one Rust package (OpenHuman's host, built from the library)
vendor/openhuman     upstream tinyhumansai/openhuman, pinned by commit on upstream/main
bin/                 spine, oh, ask, crons, seed-cron, heartbeat; bin/lib/ shared shell; bin/test/
hands/               guard, x: Python 3.11 stdlib, no dependencies; hands/lib/teeny_hands, hands/tests/
brain/               SOUL.md, CONTEXT.md, skills/, narrative/, assets/, calendar.json, cron.toml
deploy/              compose.yaml, spine.env.example, teeny.env.example, the landing page's nginx vhost
web/                 the landing page (Next.js), its own image
docs/                architecture, hands, upstream gaps
```

These are gitignored and created at run time:
- `secrets/`: the env files, the core token and the master key.
- `state/`: what the hands keep. `hands.db` holds the posting ledger, cache,
  holds and pacing, `locks/` the locks, and `core.url` where the core answers.
- `.openhuman/`: the core's config, written by the core.
- `brain/storage.db`: all of the core's state (sessions, cron jobs and runs,
  encrypted secrets) in one SQLite file, so `sqlite3` can read it.
- `brain/IDENTITY.md`, `ROLE.md`, `STYLE.md`: OpenHuman's default persona
  files. It never overwrites Teeny's `SOUL.md`.

## Tests

```sh
python3 -m unittest discover -s hands/tests -t hands/tests   # the hands; X and the core are faked
bin/test/seed_cron_test.sh                                   # cron reconciliation; no core needed
OPENHUMAN_CORE_BIN=target/release/teeny-spine bin/test/spine_live_test.sh
```

The live test boots a real spine over a scratch copy of the checkout, against
a stub model. It checks:
- first-boot minting and the file keyring;
- the single storage file, and that `SOUL.md` is left alone;
- cron seeding and a shell job running;
- `bin/crons`, `bin/oh` and `bin/ask`, and that the model receives the
  question it was asked;
- the model's context window;
- the key surviving a restart.

CI (`.github/workflows/check.yml`) builds the spine and runs all three, plus
shellcheck and the landing page's checks.

## Bumping OpenHuman

1. Move `vendor/openhuman` to the new `upstream/main` commit and run
   `git submodule update --init --recursive vendor/openhuman`.
2. Re-copy its root `[patch]` tables into `Cargo.toml`, with paths rebased
   onto `vendor/openhuman/`.
3. Run `cargo build --release -p teeny-spine`.
4. Run the live test against the new binary.
