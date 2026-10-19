# teeny

Teeny (`@teenydroid`), the TinyHumans mascot, as a stock
[OpenHuman](https://github.com/tinyhumansai/openhuman) core plus a workspace
and a handful of scripts. [`docs/architecture.md`](docs/architecture.md) is the
design. In short:

| | What | Where |
|---|---|---|
| **Kernel** | `teeny-kernel`: OpenHuman's command-line host, built here from the vendored library and run as `serve`. It provides the agent loop, cron, memory (CortexDB), browser, shell and JSON-RPC. | `kernel/`, `vendor/openhuman` |
| **Hands** | Small scripts holding every outside capability, every secret and every hard rule. Models draft; scripts act. | `bin/` (more in `hands/` as they land) |
| **Brain** | Identity, known facts, skills, narrative, assets, schedules. | `workspace/` |

Status: the kernel boots from `workspace/`, seeds its cron from
`workspace/cron.toml` and runs `bin/heartbeat`. The hands that read and post
on X come next; [`docs/hands.md`](docs/hands.md) specifies them, and
[`docs/oh-gaps.md`](docs/oh-gaps.md) lists what upstream OpenHuman could do to
make them smaller.

## Run it

```sh
git submodule update --init --recursive vendor/openhuman
cargo build --release -p teeny-kernel
mkdir -p secrets && cp deploy/kernel.env.example secrets/kernel.env   # fill in the model
bin/kernel                       # mints secrets/core.token and secrets/openhuman.key on first boot
```

In another shell:

```sh
bin/crons                        # every cron job, its next run and last result (a sqlite query)
bin/crons runs heartbeat         # one job's recent runs
bin/ask "who are you?"           # one agent turn
bin/oh cron_list                 # any JSON-RPC method on the running core
```

On a server it is the image in `Dockerfile`, run by `deploy/compose.yaml`. See
[`deploy/README.md`](deploy/README.md).

## Layout

```
kernel/              teeny-kernel, the one Rust package
vendor/openhuman     upstream tinyhumansai/openhuman, pinned by commit
bin/                 kernel, oh, ask, crons, seed-cron, heartbeat; bin/test/ for their tests
workspace/           SOUL.md, CONTEXT.md, skills/, narrative/, assets/, calendar.json, cron.toml
deploy/              compose.yaml, kernel.env.example, the landing page's nginx vhost
web/                 the landing page (Next.js), its own image
docs/                architecture, hands, upstream gaps
```

Gitignored, created by the kernel next to the checkout's own files:
- `secrets/`: the core token, the master key and `kernel.env`;
- `state/`: what jobs keep;
- `.openhuman/`: the core's rendered config;
- `workspace/storage.db`: all of the core's state, including its encrypted
  secrets and the cron jobs.

## Tests

```sh
bin/test/seed_cron_test.sh                                   # no core needed
OPENHUMAN_CORE_BIN=target/release/teeny-kernel bin/test/kernel_live_test.sh
```

The live test boots a real kernel over a scratch copy of the checkout, with a
stub model. It checks:
- first-boot minting;
- the single storage file and the file keyring;
- seeding, a shell job running, `bin/crons`, `bin/oh` and `bin/ask`;
- the key surviving a restart.

## Bumping OpenHuman

1. Move `vendor/openhuman` to the new upstream commit and run
   `git submodule update --init --recursive vendor/openhuman`.
2. Re-copy its root `[patch]` tables into `Cargo.toml`, with paths rebased
   onto `vendor/openhuman/`.
3. Run `cargo build --release -p teeny-kernel`.
4. Run the live test against the new binary.
