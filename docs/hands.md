# Hands

The only code Teeny owns. The `crates/...` paths cited below are the Rust
bot at `55c2c75`, the code each hand replaces. See [`architecture.md`](architecture.md) §3 for the
contract: one JSON object out, exit `0`/`2`/other, secrets read by the hand,
state in `state/hands.db`, `DRY_RUN=1` by default.

**R** marks a read verb. Read verbs are exposed to no-shell agents through
`hands/mcp` as read-only tools. **A** marks an acting verb, which only
`bin/job` scripts and the operator call.

## `x`: the only holder of the X keys

| Verb | | Does |
|---|---|---|
| `ingest` | A | Poll mentions since the cursor. Keep one-level-eligible mentions that aren't from the account. Insert as `pending`, unique on post id. Advance `since_id`. |
| `queue claim` | A | Atomically move one `pending` row, or a stale `replying` row with no `replyId` and a lease over 15 minutes, to `replying` with `claimedAt`. |
| `queue done/fail/hold` | A | Mark the outcome. A row that carries a `replyId` is never offered again. |
| `get`, `thread`, `user`, `search` | R | X reads through the read cache (posts 30 days; searches and threads 10 minutes). |
| `reply`, `post` | A | Run `guard check`, house style, the 280 limit (UTF-16) and the emoji cap. For a reply: the lane budget and 30–60s pacing. For an own post: the `cadence` gap under the posting lock. Then `DRY_RUN`, then X. **Record the post id first**, then the ledger, journal and memory. Never reply to the account's own post. Under `DRY_RUN`: no likes and no ledger row. |
| `upload` | A | Media upload. Returns a media id that this hand minted, recorded in `hands.db`. `post` attaches only ids from that table or a Giphy id, never a URL, and never both. |
| `article` | A | Publish an article (guard and the 48h article interval). |
| `like` | A | Like a mention (skipped under `DRY_RUN`). |
| `unsay` | A | Delete an own post. Previews unless `--confirm`. |
| `auth` | A | The three-legged OAuth flow (today's `teeny auth-x`). Operator only, interactive. |

Replaces: `crates/teeny-x/*`, `teeny-agent/src/tools/{x,post}.rs`,
`teeny/src/{mentions,stream}.rs`, `teeny/src/http/webhooks.rs`,
`teeny-store/src/mentions.rs`, `teeny-store` x-cache. The webhook and activity
stream are dropped in favour of polling, because there's no HTTP server to
receive them. Bring them back only if a 5-minute poll proves too slow.

## `guard`: the content filter

| Verb | | Does |
|---|---|---|
| `check` | R | Run, in order: links (allowlist plus approved hosts), lexicon (8 categories, leetspeak folding, word boundaries; profanity outbound only), then the judge. The judge fails closed. Returns `ok` or `hold{reason}` and writes the hold. |
| `inbound` | R | Screens third-party text before a model sees it. `bin/job` calls it before every draft. |
| `style` | R | House style: strip leading @handles, replace dashes, straighten quotes, drop NFA/DYOR. |
| `holds list/approve/reject/release` | A | Operator review. `release` atomically moves `pending` to `releasing`, publishes through `x`, and on failure goes back to `pending` with the error recorded. |

Replaces `crates/teeny-safety/*` and `teeny/src/http/{safety,release}.rs`.
Holds are written best-effort and never turn into a publish.

## `cadence`: when Teeny may post

| Verb | | Does |
|---|---|---|
| `claim <job>` | A | The 20h re-run guard, recorded at the decision, so a failed run counts. Calendar items get 1h. Also checks the gap and hot hold. |
| `slot <afternoon\|evening>` | A | Pick from the three-day rota, falling back when the owner doesn't post. |
| `gap` | R | Whether an own post may go now (7h, 20-minute tolerance, hot hold). Replies are exempt. |
| `budget <post>` | R | Reply-lane counts: Auto 10 per own post, Smart 10 per post published in the last day, Called uncapped, 7-day window, hot spill. |
| `media` | A | The daily media budget: 20 images, 3 videos (UTC). |
| `heat` | A | Sample own-post metrics and set or clear the hot hold. |

Replaces `crates/teeny-cadence/*` and `teeny-jobs/src/{heat,slot}.rs`. The 20h
guard, the pacing and the media budget are **in-memory today**. Here they are
persisted, which is a deliberate fix.

## The rest

| Hand | Verbs | Replaces |
|---|---|---|
| `market` | R: `price`, `pools`, `holders`, `global`, `ta`, `leaderboard` (rate-limited) | `crates/teeny-market/*`, `teeny-agent/src/tools/market.rs` |
| `chart` | A: `render` (spec JSON → PNG, chart and moon SVG themes) | `teeny-media` chart, moon SVG, `tools/chart.rs` |
| `media` | R: `gif`, `look` (vision through the guarded fetch); A: `gen image\|video` (under `cadence media`) | `crates/teeny-media/*` |
| `web` | R: `search`, `read`, `similar`, `answer` (Exa), `news` | `teeny-agent/src/{exa,tools/web}.rs` |
| `record` | A: `log` (journal, handles redacted for the public feed), `brief`, `health`, `export` | `teeny-jobs/src/journal_glue.rs`, `teeny/src/http/feeds.rs`, `reflections` |
| `telegram` | A: `poll` (debounced, ordered per chat), `send` (after `guard check`, honours `TELEGRAM_DRY_RUN`; the token is never logged, since it sits in the URL path) | `crates/teeny/src/telegram/*`, `teeny-jobs/src/chat` |
| `policy` | `shell`: the `beforeShellExecution` hook. Refuses anything but `hands/*` with known verbs. | `teeny-agent/src/posture.rs` DENY list |
| `mcp` | A stdio MCP server exposing only the **R** verbs above, each `ReadOnly` with no external effect | `teeny-agent/src/tools/*` registration |

**The guarded fetch** (`media look`, `web read`) inherits
`crates/teeny-media/src/fetch.rs`:
- public https only, with the URL re-checked on every redirect (at most 3);
- names resolved by the hand, refusing private, loopback, link-local and
  CGNAT addresses, with the vetted IP pinned;
- no proxy, a 20s timeout and a 10MB cap;
- a generic error back to the model.

## `state/hands.db`

| Table | From (Mongo / file) |
|---|---|
| `mentions` (queue, lease, `reply_id`, lane) | `mentions` |
| `own_posts` (ledger, heat samples, hot hold) | `own_posts` |
| `holds`, `approved_hosts` | `holds`, `holds_links` |
| `x_cache` | `x-cache` |
| `people` | `people` |
| `media` (minted ids) | in-process today |
| `guards` (20h, calendar, article interval, pacing, media budget) | in-memory today |
| `runs` (`bin/job` start, finish, exit) | — |
| `cursor` (`since_id`, `last_polled_at`) | `data/state.json` |

Dropped:
- `inferences` and `inferences_totals`. OpenHuman's traces cover them.
- `telegram_transcript`. Telegram history becomes `record/` plus memory.

`bin/migrate-mongo` copies the five live collections once, at cutover.

## Invariants every hand port must keep

Each port brings over the tests of the code it replaces. These are the ones a
port is not done without:

1. **The gap and the posting lock** (`teeny-cadence/src/cadence.rs:33,203-230`).
   `flock` plus a transaction, with the gap re-checked inside the lock.
2. **The 20h guard is marked at the decision** (`teeny-cadence/src/guard.rs`).
3. **The reply budget lanes** (`teeny-cadence/src/budget.rs`).
4. **The claim and lease, and the reply id recorded first**
   (`teeny-store/src/mentions.rs:196-209,462`, `teeny-jobs/src/reply/job.rs:464`).
5. **The atomic hold claim and release** (`teeny-store/src/holds.rs:546-572`).
6. **Dry run.** No X write, like, ledger row or memory capture
   (`teeny-x/src/publisher.rs`, `reply/job.rs:219`, `tools/post.rs:206`).
7. **Gate order and fail-closed judge**: links → lexicon → judge
   (`teeny-safety/src/gate.rs`, `judge.rs:161`).
8. **Refuse, don't fix, over 280 or over the emoji cap**
   (`teeny-safety/src/gate.rs:377`).
9. **Attachments by minted id or Giphy id only, never both**
   (`teeny-agent/src/tools/post.rs:393`).
10. **The guarded fetch** (`teeny-media/src/fetch.rs`).
11. **Secrets are never logged**, including the Telegram token in URLs
    (`teeny/src/telegram/api.rs:118-122`).
12. **No retries on posting jobs** (`teeny/src/schedule.rs:382`).
