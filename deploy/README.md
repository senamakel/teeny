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
the checks in `.github/workflows/check.yml` (lint, typecheck, build, test,
the web build; the same workflow every pull request runs) pass on the runner,
then the same script runs with `--keep-env`,
which leaves the box's two `.env` files untouched and reads what it needs to
verify from them over SSH. The workflow holds one secret, `DEPLOY_SSH_KEY`, a
dedicated key authorised for `droid@robot1`; the host key is pinned in
`deploy/known_hosts`. Changing configuration is still a deploy from a machine
that has the `.env` files.

```sh
./deploy/deploy.sh --skip-checks   # skip the local lint/typecheck/test pass
./deploy/deploy.sh --keep-env      # do not ship .env files; what CI runs
./deploy/deploy.sh --build-only    # sync and build, start nothing
./deploy/deploy.sh --logs          # tail the running stack
./deploy/deploy.sh --down          # stop it (volumes are kept)
./deploy/deploy.sh --rollback      # put the previous image back
```

## The stack

| Container          | What it does                              | Host port         |
| ------------------ | ----------------------------------------- | ----------------- |
| `teeny-bot-1`      | the agent: X ingest, the webhook, `/agent/run`, replies, the daily chart, the moon chart, Telegram | `127.0.0.1:3110`  |
| `teeny-web-1`      | the landing page, teeny.enamakel.com       | `127.0.0.1:3120`  |
| `cortex`           | long-term memory, **not this stack's**: the shared CortexDB, joined to `teeny_default` as `cortexdb` | `127.0.0.1:3141` |
| `teeny-mongo-1`    | the mention queue                          | `127.0.0.1:27020` |
| `ladder`           | the model endpoint, owned by no project    | `127.0.0.1:6969`  |
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

## The morning post

`MORNING_ENABLED=true` with `IMAGE_GEN_ENABLED=true` and `VIDEO_GEN_ENABLED=true`
in the box's `.env` (set on 2026-09-14) schedules the good-morning post at
`MORNING_CRON` (07:00 UTC since 2026-09-14, so the chart at 15:00 clears the eight-hour cadence). It draws and films through the box's own ladder,
so that ladder has to be on a router that counts Surplus's prepaid credit as
balance (llm-ladder-router #21) and have credit to spend; `ladder-up --pull`
after a router release.

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
- `brain-sync` (`deploy/brain-sync/`) pulls, stages, gates and pushes. The
  gates: `gitleaks` over the staged tree, then a grep of the staged diff for
  every value in the two `.env` files, because the ladder key has no shape a
  scanner recognises. A hit unstages and logs `REFUSED`; nothing is
  committed. `docker exec teeny-brain-sync-1 sync-now` runs a tick at once.

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

1. A write deploy key for `teenydroid/brain`, at `BRAIN_DEPLOY_KEY`:
   ```sh
   ssh-keygen -t ed25519 -N '' -C 'teeny brain-sync' -f ~/.ssh/brain_deploy_key
   gh repo deploy-key add ~/.ssh/brain_deploy_key.pub -R teenydroid/brain --allow-write -t brain-sync
   ```
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

## The memory tier is external

Teeny's memory lives in the shared CortexDB on the box: the container named
`cortex`, published on `127.0.0.1:3141` and `172.17.0.1:3141`, with its data
in the `cortex-data` volume. It is nobody's compose project here and it is
not this stack's to start or stop. It is attached to `teeny_default` with the
alias `cortexdb`, which is how `http://cortexdb:3141` resolves from the agent.
If it is ever recreated it has to be re-joined:

```sh
docker network connect --alias cortexdb teeny_default cortex
```

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
first recall rather than at boot.

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
npm run webhook                                                 # where X delivers, and who is subscribed
npm run webhook -- --set https://teeny.enamakel.com/api/webhooks/x --confirm
```

The app holds exactly one webhook, so a move is a delete followed by a create
and there is no moment where both are registered — the cron poll covers the
seconds in between. See the header of `src/scripts/webhook.ts` for the rest,
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

The cron poll is the safety net under all of it: a webhook delivery that fails
is retried for a while and then dropped, and an account that misses a mention
entirely has no way to notice.

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

`./deploy/cutover.sh --undo` reverses all of it. The script is kept because it
documents what the state actually is, not because it should ever run twice.

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
