# Deploying teeny

teeny runs on `168.100.10.208` (root, SSH key `key.pem` in the repo root;
override with `DEPLOY_HOST` / `DEPLOY_KEY`) as a small Docker Compose stack. It
takes no public port: the box's host nginx owns :80 and :443 and fronts several
projects, so everything here publishes on `127.0.0.1` only.

```sh
cp .env.example .env                 # the agent's credentials: X, Exa, Giphy
cp deploy/.env.example deploy/.env   # the infrastructure around it
./deploy/deploy.sh                   # check, sync, build, release, health check
```

```sh
./deploy/deploy.sh --skip-checks   # skip the local lint/typecheck/test pass
./deploy/deploy.sh --logs          # tail the running stack
./deploy/deploy.sh --down          # stop it (volumes are kept)
./deploy/deploy.sh --rollback      # put the previous image back
```

## The stack

| Service     | What it does                                    | Host port        |
| ----------- | ----------------------------------------------- | ---------------- |
| `bot`       | X ingest + the webhook + `/agent/run`           | `127.0.0.1:3110` |
| `bot-reply` | drains the mention queue and posts replies       | none             |
| `cortexdb`  | long-term memory                                 | `127.0.0.1:3142` |
| `tika`      | text extraction for CortexDB                     | none             |
| `mongo`     | the mention queue                                | `127.0.0.1:27020`|

Ingest and reply are two containers over one queue because they fail
differently. Retrieval has a deadline and a meter on it — X's search reaches
back only 7 days and every post retrieved is billed — so a mention missed
while the model was down is gone for good. Replying has neither, which is why
the reply half can be restarted or paused without touching the half that
cannot afford to stop.

Ports avoid what the other stacks on this box already hold: 3000 and 8080
(prmpt), 3100 and 27019 (stonks).

## The ladder

The agent and CortexDB both talk to the ladder, so there is one egress path and
one set of provider keys on the box rather than a second copy inside a
database. The ladder is a container this project does not own, and it binds
`127.0.0.1`, which a container cannot reach through `host.docker.internal`.
Two ways to point at it, both in `deploy/.env`:

- **Same box.** Leave `LADDER_URL` empty. `deploy.sh` joins the container named
  `ladder` to `teeny_default`, which is what makes `http://ladder:6969` resolve.
  Compose cannot adopt a container it did not create, which is why this is a
  script step rather than a compose entry. It has to be repeated whenever that
  container is recreated — re-running the deploy does it.
- **Another box.** Set `LADDER_URL` to something reachable *from a container
  here*. `deploy.sh` proves it answers `/models` before releasing.

Without either, `deploy.sh` refuses to deploy. That is deliberate: the agent
boots fine with no model endpoint and then fails every single turn, which from
the outside looks exactly like a healthy deployment.

**As of the last check there is no `ladder` container on `168.100.10.208`.**
It runs on the prmpt box. Either start one here or set `LADDER_URL`.

## The two .env files

`deploy/.env` is the infrastructure: database, ports, memory tier, model
endpoint. The repo-root `.env` is the agent's own credentials and persona
settings. Both are excluded from the rsync and copied explicitly, so a missing
local file can never wipe a working one on the box.

`CORTEX_API_KEY` (deploy/.env) and `CORTEX_TOKEN` (.env) are the same secret
from two sides — the service's own key, and the client's copy of it. They are
checked against each other before every deploy, because a mismatch fails at the
first recall rather than at boot.

`DRY_RUN=false` in `.env` is the only thing between a drafted reply and a
published one. The deploy prints which of the two it is shipping.

## Before the first real deploy

1. **X credentials.** `X_ACCESS_TOKEN` and `X_ACCESS_TOKEN_SECRET` come from
   `npm run auth:x`, not the developer portal, and must be authorized while
   logged in as the bot account.
2. **Embedding dims are permanent.** `CORTEX_EMBEDDING_MODEL` and
   `CORTEX_EMBEDDING_DIMS` are pinned on CortexDB's first ingest. Changing
   either afterwards orphans everything already stored.
3. **The mention queue is not derived data.** Unlike the stonks database, it
   cannot be rebuilt: X's search only reaches back 7 days. Migrate the
   `mongo-data` volume, never recreate it.

## The webhook (optional)

The cron poll finds mentions on its own, so the stack is useful with no public
address at all. The webhook is the fast path — a mention arrives in seconds and
costs nothing against the post-read cap — and it needs public HTTPS:

```text
GET/POST https://<host>/webhooks/x
```

`deploy/nginx/teeny.conf` is the vhost for it. Install it, put a real hostname
in it, issue a certificate, then subscribe the account to the webhook in the X
developer console. `/health` and `/queue` are deliberately not published; read
them over SSH on `127.0.0.1:3110`.

## The box

Kernel 7.0.0, 7.9GB RAM, 4GB swap. Mongo is pinned to `7.0` because 8.2
segfaults in a loop on this kernel (`die 139`, reproducible with an empty
volume and no client) and 8.0 refuses to start at all (SERVER-121912).

```sh
ssh -i key.pem root@168.100.10.208
docker compose --project-directory /root/teeny/deploy \
  -f /root/teeny/deploy/compose.prod.yaml --env-file /root/teeny/deploy/.env ps
curl -sS http://127.0.0.1:3110/queue   # the backlog, by status
```
