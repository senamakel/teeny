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

## The chart agent

A second agent on its own timer. Once a day it reads the token's chart, renders
a candlestick image, and posts one post with it attached. It shares the persona
and the same CortexDB memory as the reply drafter, so the voice is one voice and
yesterday's post is something it can actually remember writing.

It is off by default (`CHART_ENABLED=false`), because it publishes.

```sh
npm run chart-dry-run              # the whole agent, dry: research, draft, render, no post
npm run chart-dry-run -- --data-only   # just the market read and the PNG
npm run chart-dry-run -- --publish     # actually posts, if DRY_RUN=false
```

### Where the numbers come from

No single provider answers everything, so three sit behind `TokenMarket` and it
merges them:

| Source | What it gives | Notes |
| --- | --- | --- |
| Dexscreener | price, market cap, liquidity, 5m/1h/6h/24h change, buy and sell counts | no OHLCV endpoint |
| GeckoTerminal | OHLCV candles, per-trade wallets | free, no key |
| Solana RPC | supply and holder concentration | public RPC rate-limits; set `SOLANA_RPC_URL` |
| GMGN | KOL and smart-money labels, ranked traders | **unreachable**, see below |

GMGN would have the best of it — smart-money tags, KOL wallets, top-trader PnL
— and has no public API. `gmgn.ai` fronts everything with Cloudflare's bot
check, which answers a server with an HTML interstitial and a 403 whatever
headers it sends; verified against both the token-quotation and the candles
endpoints. So
`GmgnClient` exists, is tried first, and stays off unless `GMGN_BASE_URL` names
something that will actually answer. With it unset the `ranked_traders` tool
returns "no ranked-trader source is configured" and the prompt forbids claiming
any KOL activity — which is the correct behaviour, not a degradation.

One live-data bug worth keeping in mind: pools are ranked by 24-hour **volume**,
not liquidity. $TINY's deepest Solana pool holds $139K and did $336 of volume in
a day, while the pool everybody actually trades holds $93K and did $10K.
Charting the deepest one drew a flat line through a pool nothing happens in.

### The technical read

`src/ta/` computes it: EMA, Bollinger, ATR, VWAP, fractal swing points, and
support/resistance clustered by relative distance so it works the same on a token
priced at $0.0007 as on one priced at $40,000. Every series returns `undefined`
for its warm-up rather than a zero or the first close, so a chart never draws a
trend line through a period the indicator could not see.

**There are no oscillators.** RSI and MACD were written, tested, used, and then
removed. They are mean-reversion tools for instruments that mean-revert, and a
memecoin does not: RSI sat pinned at 74 reading "overbought" through exactly the
kind of move the account exists to post about. In a post that is worse than
useless — it is a caveat that argues holders out of their own thesis over a
number that never applied here. `test/analysis.test.ts` asserts the read computes
none, and the prompt forbids the words, because a model will happily supply
"overbought" from memory if the tools stop saying it.

What replaced them is what actually moves one of these. Levels are only levels
if price has been to them more than once — a line touched once is a coordinate,
and "broke resistance" about one of those makes an account unreadable. When one
is crossed, `analyse` reports it as a **breakout**, a **breakdown** or a
**reclaim** (crossing back over something previously lost, which reads completely
differently to somebody who held through losing it), notes how many bars ago, and
marks whether the bar that did it traded above average volume — a break nobody
traded is a wick that gets given straight back, so it is downgraded rather than
posted as news. Volume is read twice: the last bar against its average, and the
recent half of the window against the half before it.

`readProgress` in `src/ta/progress.ts` is the long horizon, on daily candles:
change over 7, 30 and 90 days, how many days closed green, distance from the
daily high and low, and whether the daily lows are stepping up. No indicators at
all — on a daily memecoin series there is not enough history for one to mean
anything, and the honest long-term signals are arithmetic. A token three weeks
old falls back to whatever history it has rather than reporting nothing.

`analyse()` turns that into signals, notes, and a `bullishScore` out of 100. The
score reports bearish signals honestly even though the account is unapologetically
long: without the window-trend signal, a token down a third over five days scored
bullish on a two-hour bounce, and the post would have read "structure intact"
under a chart that was visibly bleeding. Under 40, the prompt tells the agent to
post about accumulation, distribution or what shipped this week instead of
finding something nice to say about the price.

### Four ways of looking at the same chart

`chart_timeframes` reads the token hourly, 4-hourly, daily and weekly in one
call, and reports whether they agree. One timeframe is one opinion: an hour can
be red inside a green week and both are true, and a post built on whichever
timeframe happened to be the config default is a post whose framing was chosen by
an env var. Each horizon gets its own trend, structure, level event, support and
best positive, and the tool ends with the alignment — all four bullish is itself
the story, none of them bullish means the positive is on-chain today.

Weekly candles do not exist upstream. GeckoTerminal's daily endpoint accepts
`aggregate=1` and answers 400 to anything larger, so `fold()` builds them here
from daily bars: first open, last close, the extremes, volume summed. It buckets
on absolute time rather than counting back from the newest bar — counting back
would silently redraw every weekly candle depending on what hour the job ran.

The market layer caches each series, so reading four timeframes and then
rendering one costs four fetches rather than five. That is not an optimisation:
GeckoTerminal's free tier started answering 429 on the first multi-timeframe run
and the post lost its holder data to it.

### The market as the frame

`market_context` reads BTC, ETH, SOL, total market cap and BTC dominance from
CoinGecko and computes the token's outperformance in percentage points over the
same 24 hours. This is the single most useful thing the account can say on a bad
day, and it is a fact rather than a spin: a token up 14% while the whole market
is down 1.8% is genuinely outperforming, and "everything is bleeding and we are
not" is more informative to a holder than the 14% on its own.

Comparisons are only offered when the token actually won them. On a day it lagged
everything, the tool says so and tells the agent to take its positive from the
chart, the holders or the longer horizon instead — the point is to choose among
true comparisons, not to manufacture one.

`crypto_news` is the same idea qualitatively: Exa search bounded to the last few
days, so the post can place itself against the market's weather. `web_search`
could do this and does it badly for a daily post — without a date bound it
happily returns an analysis piece from March, and framing today against last
spring is worse than ignoring the news entirely.

### Always leading with the positive

Both `analyse` and `readProgress` return a ranked list of true positives, handed
to the agent as `TRUE POSITIVES YOU CAN LEAD WITH, best first`. It is never
empty — the last entry is always price's distance above its own low, which is
true of every chart that has ever existed. Order matters more than contents: a
defended level beats a percentage, a percentage beats a ratio, now beats last
week.

The three framings the prompt reaches for, in order: **relative strength** (the
market is red, we are not), **a new base** (a level that broke is a story about
where price stopped and what it is standing on now, not about losing a level),
and **the longer horizon** (an hour can be red inside a green week). `findBase`
only reports a base once price has actually held a floor for three bars and the
last bar did not set the low — calling a base mid-fall is the easiest way for an
optimistic account to be wrong in public.

This is structural because prompting alone did not hold. Left to choose its own
framing from a mixed read, the agent opened a post with `rough hour: we lost
$0.0007292` — accurate, and the wrong first sentence to put in front of people
who hold the token. The prompt now bans opening on a loss, volunteering a bearish
caveat, and the words "rough", "ugly" and "bleeding" about the chart; a broken
level may appear mid-post but never as the headline or the last word.

None of that permits saying something untrue. Every positive is computed from the
same candles as everything else — what changed is which true thing goes first.
There is also a cap of three numbers per post, and percentages off the low are
banned as headline stats: the first post to lead with the long horizon read
"+702%, +3394%, 3718% off the daily low", which is a spreadsheet screenshot and
looks exactly like a pump.

### Rendering

The default renderer draws the chart here: SVG, rasterised to PNG by resvg.
TradingView's dark palette, EMA20/EMA50, a volume panel, the support and
resistance the analysis found, and the account's handle as a watermark. The SVG
is a string, so the tests assert on it and a diff shows what changed.

Two details that are not obvious from the picture. The volume panel is scaled to
the 95th percentile, not the maximum, because one launch bar is routinely fifty
times the median and scaling to it flattens every other bar into the axis; bars
over the cap clip to the top of the panel, which reads correctly. And the price
scale includes the overlays, because an EMA that leaves the panel reads as the
price having gone somewhere it did not.

resvg is the only native dependency in the repo. It draws text through the
system fonts, and alpine ships none — hence `apk add font-noto` in the runner
image. Without it the chart renders and every label silently disappears.

`CHART_RENDERER=screenshot` is the other option: drive a headless browser at a
page that draws a chart and photograph it. It is not the default because DEX
Screener's embed does not serve its chart data to a headless browser — the page
loads, the TradingView shell and toolbar render, and the panel says "No data
here", verified against the $TINY PumpSwap pool with a real Chrome binary and
generous waits. The URL is a template, so the useful version of the idea is
pointing it at a page we serve ourselves; that needs a browser in the runtime
image, which is why it is opt-in.

### Who it is writing for

The post is addressed to $TINY holders and the people around the project, not to
a general crypto audience — and the prompt says so, because the difference shows
up in every line. It holds the token, so "we" is honest; it says what today means
for people already in rather than what a stranger should do; and on a red day it
shows up and levels with them instead of going quiet, which is the thing holders
actually remember.

Two guardrails come with that audience. No promises of any kind — no giveaway, no
airdrop, no hint of one; the last one cost the account its reach and holders wore
the reputation for it. And at most one handle in a post, only for something
visible in a tool result, because a broadcast post with a list of handles in it
is engagement bait and reads as one.

### Memory, on both sides of the post

`ChartHarness` wraps the agent with the memory work rather than leaving it to the
model. Before the turn it recalls two things, every run: what this account has
already posted (so today's angle is not yesterday's) and who the community is —
who has been showing up, what they have been asking, what the mood is. Both are
already in the task when the agent starts.

That is deterministic for a reason. Left to `recall_memory`, one run called it
twice with near-identical queries and burned two turns, and nothing stopped a run
from skipping the community entirely and posting a technical read into a void.
Recall failing is not a reason to skip the day, so a failure degrades to "nothing
recalled — treat today as your first post" and the post still goes out.

Afterwards, `post_chart` writes the published text back through `captureChartPost`
— the text itself, not the model's summary of it, since "do not repeat yesterday's
angle" cannot be judged against a paraphrase. Dry runs are captured too and
labelled unpublished, so a rehearsal leaves a trace without poisoning tomorrow. If
memory is down at that point the post has already gone out, so the failure is
swallowed rather than raised: reporting it would read to the agent as a failed
publish and invite it to post again.

`community_pulse` is the live half — one X search for the cashtag with the
account's own posts filtered out, so the post can answer something people are
already talking about. On a quiet day it says the day was quiet, which is a fact
about the day rather than an invitation to invent enthusiasm.

### Tools

| Tool | What it does |
| --- | --- |
| `token_overview` | price, market cap, liquidity, 24h volume, change, buy/sell counts, and the source |
| `token_candles` | raw OHLCV, for when a post needs one specific bar |
| `market_context` | the token against BTC, ETH, SOL and the whole market over the same 24h |
| `crypto_news` | recent crypto headlines, date-bounded, for what kind of day the market is having |
| `chart_timeframes` | the same chart hourly, 4-hourly, daily and weekly, and whether they agree |
| `technical_analysis` | the read on one horizon: levels, breakouts and reclaims, volume, structure, bullish score |
| `token_progress` | the long horizon on the daily: 7/30/90-day change, green days, stepping lows |
| `render_chart` | draws the chart, returns a `chartId` |
| `token_holders` | supply and concentration from the chain |
| `token_flow` | buy versus sell volume, top buyers and sellers, watchlist wallets |
| `trader_activity` | one wallet's footprint in the recent-trades window |
| `ranked_traders` | KOL and smart-money labels, when a provider is configured |
| `community_pulse` | what holders have actually been posting about the token lately |
| `post_chart` | publishes one post with a `chartId` attached. The only way it publishes |

It also gets `recall_memory` and `remember`, and read-only X search. It cannot
reply and it has no `x_post`.

Two things are bound in configuration rather than left to the model. The token
address, because an agent that can be told which contract to analyse can be told
to analyse somebody else's by anything that lands in its context — and this one
reads posts. And publishing, which goes through `post_chart` alone: one post,
one image, the emoji cap and the length limit enforced in code.

Wallet addresses are shown truncated to the agent and the prompt forbids putting
one in a post. So is any URL: links cost an account its reach, and this one has
already paid for that lesson once. The chart is the attachment.

The job refuses to run twice inside 20 hours even if the cron fires again — a
container that restarts near the cron minute would otherwise post the same chart
an hour later, which is exactly the repetitive-publishing shape that got this
account throttled.

## The journal, and the mind page

Every turn the account takes is written to a journal, and the journal is what
[teeny.enamakel.com](https://teeny.enamakel.com) shows: what came in, what
memory said, which tools were called and what they returned, the drafter's
reasoning in its own words, what it said, what it decided against, and how it
ended (`posted`, `dry-run`, `skipped`, `held`, `failed`).

The journal is not memory. Memory is what the account carries into the next
conversation and is deliberately selective: sour posts get an answer and no
memory. The journal is the audit trail and keeps everything, including the
turns where it stayed quiet. It lives in the same Mongo as the mention queue
(`reflections` collection), every role writes to it, and the ingest role
serves it at `GET /mind` with every handle blanked before it leaves. Writing
it is best-effort: a journal that cannot be written is logged and the reply
still goes out.

The reasoning comes from the drafter itself. Its structured output carries
`thinking` and `declined` alongside the reply, and neither changes what gets
posted; they are the record of how it got there.

## The wallet, and why there isn't one

The account was given an ERC-20 wallet on Robinhood Chain on 2026-09-08 and had
it taken away the same day. `src/airdrop/` and `src/clients/wallet.ts` are still
here and still tested — the spend policy, the ledger, the recipient registry and
the ERC-20 client — but nothing constructs them and no tool reaches them, so the
agent cannot move funds.

It is worth being precise about what failed, because the machinery did not. Over
one campaign it sent 106 airdrops and the limits all held: the amount cap, the
daily budget, one-per-account, the rule that an address must appear in the post
being answered, and the check that stops one wallet collecting from twenty fresh
accounts. Nothing was overpaid and nothing was lost.

What broke was the account. A hundred near-identical reply-and-pay posts in an
hour is the shape of spam however sincerely they are meant, and X throttled the
account's reach for it. The lesson is not about ceilings. It is that a giveaway
run at speed in public costs the thing the account exists for.

`test/capabilities.test.ts` holds the long version, and fails if a spending tool
is ever wired back in.

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
npm run chart-dry-run   # rehearse the daily chart post without publishing
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
