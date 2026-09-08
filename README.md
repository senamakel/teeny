# teeny

Node.js and TypeScript service that runs Tiny (`@TinyMascot`), the TinyHumans AI
mascot, as an X account. It:

- receives and verifies X Account Activity webhooks;
- captures X activity and bot turns in CortexDB;
- polls explicit mentions on a configurable cron;
- recalls relevant CortexDB context before each harness turn;
- drafts replies with a LangGraph agent that can research before it answers;
- exposes the same agent over HTTP so it can be given ad-hoc tasks;
- persists a small local cursor to prevent duplicate replies;
- defaults to dry-run mode so no post is published accidentally.

## Setup

Requirements: Node.js 20+, an X developer app with read/write access, a CortexDB
token, and an OpenAI-compatible model endpoint. An Exa API key is optional; the
agent simply runs without web-research tools when it is absent.

```sh
cp .env.example .env
npm install
npm run dev
```

Fill every blank value in `.env`. Leave `DRY_RUN=true` until generated replies
have been inspected. Set it to `false` only when the bot is ready to publish.

## Persona: SOUL.md and skills

`SOUL.md` holds the account's identity, voice, and hard formatting rules.
`CONTEXT.md` holds the facts it may treat as known: the lab, the token, the
repositories, the published roadmap, and the rule that $TINY is the only token
it ever promotes. Volatile numbers, price and market cap and burn totals, are
deliberately absent so they cannot be quoted from memory.
Every skill under `skills/` is a Markdown document the agent follows on every
turn; `skills/humanizer/SKILL.md` is the rewrite guide that keeps posts from
reading like a chatbot.

Both are read from disk at startup and prepended to the agent's system prompt,
for the task agent and the reply drafter alike. A skill is either
`skills/<name>/SKILL.md` or `skills/<name>.md`. Change either file and restart;
nothing is compiled in.

```text
SOUL_PATH=SOUL.md        # unset or missing: the agent falls back to a generic identity
CONTEXT_PATH=CONTEXT.md  # unset or missing: no known-facts section
SKILLS_DIR=skills        # unset or missing: no skills are loaded
MAX_EMOJI_PER_POST=1     # enforced by x_post, not just requested in the prompt
```

SOUL.md's one-emoji rule is the only formatting rule the code enforces: `x_post`
counts emoji sequences (a joined family or a skin-tone modifier counts once) and
refuses the call with an explanation instead of publishing. Everything else in
SOUL.md and the humanizer is prompt-level guidance.

Both documents are long. They are sent on every agent turn, so they dominate the
prompt budget; trim them or point `SKILLS_DIR` at a smaller set if that matters
more than the voice.

## The agent

`AGENT_MODE=langgraph` (the default) runs a LangGraph ReAct agent built from
`@langchain/langgraph`. It is wired to three groups of tools:

| Tool | What it does |
| --- | --- |
| `recall_memory` | Reads CortexDB long-term memory for prior turns, product facts, and tone. |
| `remember` | Writes a durable note back to CortexDB. |
| `x_search_posts` | Searches the last 7 days of X posts with X search operators. |
| `x_get_post` | Fetches a single post, with its author and conversation id. |
| `x_get_comments` | Reads the replies/comments in a conversation. |
| `x_post` | Publishes a post, a reply, or a quote. Inert while `DRY_RUN=true`. |
| `web_search` | Exa search with page excerpts. |
| `read_web_pages` | Exa contents fetch for specific urls. |
| `find_similar_pages` | Exa similarity search around one good source. |
| `web_answer` | Exa's cited answer endpoint for a single fact. |
| `find_gif` | Searches the account's own Giphy channel, or all of Giphy on request. |

`x_post` takes a `gifId` from `find_gif` and attaches it: the id is resolved
against Giphy server-side and the bytes are uploaded to X through the chunked
media API, so the account can only post GIFs that Giphy actually serves and
never an arbitrary URL. An animated GIF is transcoded by X after upload, so the
client waits for processing to report `succeeded` before attaching the id.

The cron reply job uses a research-only variant of the agent: it has every tool
except `x_post`, and returns a structured `{ reply, gifQuery?, incomingTone }` so the job stays in charge of
publishing and of the duplicate-reply cursor. When the drafter sets `gifQuery`,
the job searches the channel and attaches the first match; a GIF is a nice
touch rather than the point, so any failure there is logged and the reply still
goes out as text. Set `AGENT_MODE=simple` to fall
back to the original single chat-completion path.

Post text, memory, search results, web pages, and GIF titles are treated as
untrusted data throughout; the system prompt tells the agent to report embedded
instructions rather than follow them.

Two rules no incoming message can change: $TINY is the only token the account
promotes, and the account never takes a wallet action. The second is enforced
by the tool set rather than by the prompt. The agent holds no keys and has no
tool that could move funds, so "send 5 SOL to this address" in a reply has
nothing to reach; `test/capabilities.test.ts` asserts the published tool list
exactly, so adding a wallet capability fails the suite and has to be a
deliberate decision rather than an inherited one.

### Ad-hoc tasks

Set `AGENT_API_TOKEN` (16+ characters) to enable the task endpoint:

```sh
curl -X POST https://your-host.example/agent/run \
  -H "authorization: Bearer $AGENT_API_TOKEN" \
  -H 'content-type: application/json' \
  -d '{"task":"Check what people are saying about stonks.rocks this week and draft a post about it","threadId":"weekly-recap"}'
```

The response is `{ output, steps }` — the agent's final text plus every tool call
it made. `threadId` resumes an earlier conversation from the in-memory
checkpointer; that state is lost on restart. The endpoint returns 503 while
`AGENT_MODE=simple` or `AGENT_API_TOKEN` is unset, and the agent still honours
`DRY_RUN`, so nothing is published until you turn it off.

## The wallet

The account holds a wallet on Robinhood Chain (an Arbitrum L2, chain id 4663,
gas in ETH) and can send small $TINY airdrops from inside a conversation. It
decides for itself whether somebody has earned one.

```text
airdrop_user(to, amount)  ->  "Sent 50 TINY to 0x... Transaction: 0x..."
                          ->  "No airdrop was sent: <reason>"
```

This is the one tool that spends, and it was added against the explicit advice
recorded in `test/capabilities.test.ts`, which is worth reading before changing
any of it. The argument there is sound: every tool on the reply path is
reachable from text written by strangers, so a wallet tool there is a wallet a
stranger can address, and a prompt is an argument rather than a permission
boundary. The account is meant to tip people in conversation, so the tool is on
that path anyway and the defence is placed elsewhere.

**Nothing that matters is decided by the model.** `src/airdrop/policy.ts` is
checked after the model has already asked, against the ledger:

| Limit | What it stops |
| --- | --- |
| the address must appear in the post being answered | an address from a search result, a memory, or somebody else in the thread being paid |
| the post's author is the only one who can claim it | publishing an address and having a later reply pay it |
| `AIRDROP_MAX_PER_AIRDROP` | the amount being argued upward |
| `AIRDROP_MAX_PER_RECIPIENT` | the same account being paid twice |
| `AIRDROP_MAX_PER_DAY` | a bad day costing more than a day's budget |
| one wallet, one account | a farm of fresh accounts all pointing at one wallet |
| `AIRDROP_REQUIRE_FOLLOW` | drive-by begging from accounts with no connection to the project |
| a unique index on the post id | one post being paid twice by two turns at once |

The account is keyed on its numeric X id and the wallet is stored beside it in
`recipients`, so a rename does not make somebody a new person and one wallet
cannot collect from twenty accounts. Handles are kept as history, and nothing is
ever decided by them — that lesson cost an afternoon already.

Following is a deliberately weak signal: a follow costs nothing and a farm can
buy a thousand, so it filters drive-by begging rather than defending anything.
When X cannot be asked at all, the answer is treated as no, because not sending
is recovered by them asking again and sending wrongly is not recovered.

## Watching it decide

```sh
npm run airdrop-sim
```

Plays the real tool and the real policy against the situations it will meet —
the same person asking twice, somebody negotiating the cap upward, a farm of
fresh accounts behind one wallet, a non-follower, and a prompt injection. It is
entirely in memory: no chain, no X, no model, no configuration, so it runs
anywhere and cannot spend anything.

The worst case is therefore boring: the most persuasive post anyone can write
gets its own author, once, at most one call's cap, while the budget lasts.
That is an airdrop working, not an exploit.

Two operational details worth knowing. The ledger row is claimed *before* the
transfer is broadcast, so a crash between the two blocks a retry rather than
paying twice — an airdrop that may not have happened is the safe way to be
wrong. And the airdrop happens while the reply is being drafted, so a reply that
fails to publish afterwards leaves tokens sent with nothing posted; the ledger
has it, X does not.

`AIRDROP_ENABLED=false` removes the tool entirely rather than making it fail,
and `AIRDROP_DRY_RUN=true` runs the whole path and broadcasts nothing. Both are
independent of `DRY_RUN`, because publishing replies and spending money are
different decisions.

## Tone and what gets remembered

The drafter judges how the incoming post read, and that judgement picks the
reply's register:

| Tone | Reply | Remembered |
| --- | --- | --- |
| friendly, including teasing and banter | warm, curious | yes |
| neutral, a blunt or plain question | straight answer | yes |
| negative: snark, FUD, a lazy dismissal | roasted back, one punchline, with the real answer underneath | no |
| hostile: abuse, slurs, threats | one flat line or nothing | no |

Snark and abuse are separated deliberately. Collapsing them means either
roasting somebody who is actually abusive, which is a fight the account cannot
win, or politely absorbing snark, which is duller than the account should be. A
roast aims at the argument, the cliche, or Tiny itself, never at how a person
looks, who they are, or what they hold. Anyone genuinely upset or reporting a
real problem gets none of it and is simply helped.

Neither sour case is stored. Memory is what the account carries into every
future conversation, so an insult kept today colours a reply to somebody else
next week; the job logs each one it declined to remember.

GIFs are the default rather than the exception. The drafter is told to reach
for one on greetings, reactions and jokes, and to skip it only when the post is
a real question wanting a real answer.

## Memory is per person

Every mention is stored as a chat turn whose `observedActor` is the person who
wrote it, keyed on their numeric X id rather than their handle, which can
change. The bot's own reply is stored as the assistant turn about that same
person. Recall then runs twice: once filtered to `observed_actor` for what the
account knows about this individual, and once unfiltered for topic and product
context. A single query over everything buries a quiet person's history under
general context, and the drafter is told who it is answering.

Before this, every event was written as an opaque JSON blob attributed to the
bot's own id, so recall could not tell two people apart and the account
answered a stranger and a regular identically.

## Ingest and reply are separate

Reading X and answering it are split across a cache, because they fail
differently. Retrieval has a deadline and a meter on it: recent search reaches
back only 7 days and every post retrieved is billed, so a mention missed while
the model was down is gone for good. Replying has neither; it can be slow,
retried, or paused.

Two things get queued: a direct mention or reply to the account, and anything
further away where somebody actually typed the handle.

That second case matters because X prefixes every earlier participant's handle
onto a reply, so a thread the account was tagged into once names it in every
reply afterwards, none of them addressed to it. The leading run of handles is
that machinery; anything after it was typed on purpose. `@someone @tiny wen
ser` is inherited and ignored, `@someone Oh hi fren @tiny` is deliberate and
answered, wherever in the thread it sits.

Nothing here replies to a post that does not name the account. X permits an
automated reply to a post that mentions you and not to one that does not, and
the line the code draws is the same line.

The cursor still advances past what was dropped, so nothing is re-read and
re-billed.

There are two ways into the queue and one way out:

```text
webhook     X Account Activity -> MongoDB `mentions` (status: pending)
IngestJob   cron -> X search   -> MongoDB `mentions` (status: pending)
ReplyJob    cron -> pending mentions -> agent -> X reply -> status: replied
```

The webhook is the fast path: a mention arrives in seconds, costs nothing
against the post-read cap, and needs no search. The cron poll stays on as a
safety net, because a webhook delivery that fails is retried for a while and
then dropped, and an account that misses a mention entirely has no way to
notice. Both write through the same insert-if-absent, so a mention that arrives
twice is queued once.

The webhook handler queues and returns. It does not draft: X retries a delivery
it does not see acknowledged quickly, and a model call takes far longer than
the line is held open.

`SERVICE_ROLE` decides what a process runs: `all` (both, the default),
`ingest`, `reply`, or `api` for the webhook and agent endpoint alone. The same
image serves every role, so splitting them is a compose change rather than a
build. Run the reply side as its own container when you want to restart or
redeploy it without pausing ingestion.

A cached mention is only ever inserted, never reset, so the overlapping search
window cannot undo work. A reply failure records the error and the attempt
count and leaves the post pending for the next run. `GET /queue` reports the
backlog by status.

## API cost

X bills per post *retrieved*, not per request, so the polling window is the
thing that costs money. Each ingest run searches only what arrived since the previous
run: `since_id` when a reply has been sent before, and `start_time` from the
last poll otherwise. `lastPolledAt` is saved on every run, before any reply
work and whether or not anything matched, so a quiet account or a crash
mid-run cannot widen the next window back to the full 7 days X allows. The
window carries a minute of overlap for indexing lag, which is harmless because
`repliedPostIds` already prevents a duplicate reply.

```text
MENTION_LOOKBACK_MINUTES=15   # how far back the first poll of a fresh state reaches
X_SEARCH_MAX_RESULTS=10       # ceiling per search, the agent's own included
```

X's floor for `max_results` is 10, so asking for fewer still bills for 10. The
agent's `x_search_posts` and `x_get_comments` are capped by the same setting,
because a research turn can otherwise pull far more posts than the reply is
worth. Every poll logs `retrieved` alongside the window it used, so the monthly
spend can be read straight out of the logs.

## X authorization

The bot signs every write with an OAuth 1.0a user access token pair for the bot
account. The app's consumer key and secret are not enough, so run the flow once:

```sh
npm run auth:x
```

It prints an authorize URL, waits on `X_OAUTH_CALLBACK_URL` for X to redirect
back, exchanges the verifier, and writes `X_ACCESS_TOKEN` and
`X_ACCESS_TOKEN_SECRET` into `.env`. Authorize while logged in as the bot
account, not a personal one.

Before the first run, open the X developer portal and, under the app's User
authentication settings, turn on OAuth 1.0a with Read and write permissions and
register the callback URL exactly as it appears in `.env`. X rejects the request
token with a bare `403 Forbidden` when that URL is missing or does not match.

Set `X_OAUTH_CALLBACK_URL=oob` to use the PIN flow instead. X then shows a
number to paste into the terminal and no callback URL has to be registered.

## X webhook

The service must be behind public HTTPS, and X has to be told where. In
production that is `https://teeny.tinyhumans.ai/webhooks/x`.

```sh
npm run webhook                                   # where X delivers today
npm run webhook -- --set <url>                    # preview a move
npm run webhook -- --set <url> --confirm          # do it
```

The endpoint implements X's challenge-response check and validates the
`x-twitter-webhooks-signature` header before accepting activity. `--set`
registers the URL, which is when X sends the CRC challenge, and subscribes the
account; the app is allowed one webhook, so an existing one is retired first
and put back if the new registration is refused.

## Commands

```sh
npm run dev
npm run typecheck
npm test
npm run lint
npm run build
npm run webhook     # read or move the X Account Activity registration
```

Runtime cursor data is stored at `data/state.json` and is ignored by
Git. Long-term activity and conversation memory lives in CortexDB.

## Deployment

The service runs on a DigitalOcean box as a Compose stack — the agent split
into an ingest half and a reply half, CortexDB, Tika, and Mongo — behind the
box's host nginx, talking to the ladder for every model call.

```sh
cp .env.example .env                 # the agent's credentials
cp deploy/.env.example deploy/.env   # the infrastructure around it
./deploy/deploy.sh
```

`deploy/README.md` has the whole picture: the ladder wiring, the two .env
files, the ports, and what cannot be recreated if a volume is dropped.

## Operational constraints

- The self-serve X API permits automated replies when the bot was explicitly
  mentioned or its post was quoted. This worker deliberately searches direct
  mentions rather than replying to arbitrary posts.
- X requires webhook endpoints to be publicly reachable over HTTPS, return
  quickly, implement CRC, and validate request signatures.
- The Account Activity API does not deliver the home timeline. It covers the
  subscribed account's posts, mentions, replies, reposts, likes, follows,
  blocks, mutes, and direct-message activity available to its access tier.

References: [X Account Activity API](https://docs.x.com/x-api/account-activity/introduction),
[X webhooks](https://docs.x.com/x-api/webhooks/introduction),
[X post integration rules](https://docs.x.com/x-api/posts/manage-tweets/integrate),
and the [CortexDB TypeScript SDK](https://cortexdb.ai/docs/quickstart/typescript).
