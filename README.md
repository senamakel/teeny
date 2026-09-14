# teeny

Node.js and TypeScript service that runs Tiny (`@TinyMascot`), the TinyHumans AI
mascot, as an X account. It:

- receives and verifies X Account Activity webhooks;
- captures X activity and bot turns in CortexDB;
- polls explicit mentions on a configurable cron;
- recalls relevant CortexDB context before each harness turn;
- drafts replies with a LangGraph agent that can research before it answers;
- exposes the same agent over HTTP so it can be given ad-hoc tasks;
- talks on Telegram as the same account, in DMs and in groups it is named in;
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
turn. There are three: `humanizer` is the rewrite guide that keeps posts from
reading like a chatbot; `humor` is the joke mechanics, the sarcasm dial and the
roast recipe; `video-script` is how the morning clip is scripted as a
ten-second sketch a video model can render.

Both are read from disk at startup and prepended to the agent's system prompt,
for the task agent, the reply drafter, and the morning post's pick, script and
caption stages alike. A skill is either
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
| `look_at_image` | Describes an image by URL through the vision ladder. Off when `VISION_MODEL` is blank. |
| `generate_image` | Draws a picture, with Tiny's look from the reference sheet, and returns a `mediaId`. `IMAGE_GEN_ENABLED`; present on a calendar item that carries an image, on an X reply that explicitly asked, on a task; never on Telegram. |
| `generate_video` | Films a few seconds, same sheet, and returns a `mediaId`. `VIDEO_GEN_ENABLED`; same gating. |

`x_post` takes a `gifId` from `find_gif` and attaches it: the id is resolved
against Giphy server-side and the bytes are uploaded to X through the chunked
media API, so the account can only post GIFs that Giphy actually serves and
never an arbitrary URL. An animated GIF is transcoded by X after upload, so the
client waits for processing to report `succeeded` before attaching the id.

The cron reply job uses a research-only variant of the agent: it has every tool
except `x_post`, and returns a structured `{ reply, gifQuery?, mediaId?, incomingTone }` so the job stays in charge of
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

## Pictures

The account can look at images, draw them, and film short clips, on X and on
Telegram alike. All three default to the LLM ladder, which serves them on the
same URL and key as chat (`llm-ladder-router` PR #18): a `vision` chat ladder
whose rungs read image input, an `image` ladder in the OpenAI Images shape,
and a `video` ladder that answers with a job to poll. Seeing is on by default;
drawing and filming are opt-in because each is a bill. Any part can be pointed
at another OpenAI-compatible endpoint with its own URL, key and model.

| Capability | Switch | Default | Shape |
| --- | --- | --- | --- |
| Seeing | `VISION_MODEL` (blank = off) | `vision` on the ladder | chat completions with `image_url` parts |
| Drawing | `IMAGE_GEN_ENABLED` | `image` on the ladder | `POST /images/generations`; also `gpt-image-*` / `dall-e-*` directly |
| Filming | `VIDEO_GEN_ENABLED` | `video` on the ladder | `POST /video/generations` + polled `media.job`; `VIDEO_GEN_API=openai` for Sora's `/videos` |

With nothing enabled beyond the default vision, the tool list gains only
`look_at_image`; `test/capabilities.test.ts` asserts exactly which tools appear
with which provider.

**Seeing** happens before drafting rather than on request. A photo sent to the
bot on Telegram is downloaded and described, and the description lands in the
message text as `[image: …]`, so the transcript, the drafter and memory all
see the same thing. A picture attached to an X mention is fetched by the media
URL X returns (a video contributes its preview frame) and passed to the drafter
as a labelled, untrusted block. Without a vision model the message reads
`[sent a photo]` and the prompt tells the account to say it cannot see it
rather than guess. In a group, only photos addressed to the bot are described:
a vision call per picture in a busy room is a bill for pictures nobody asked
about. `look_at_image` covers everything else -- the media URLs listed by
`x_get_post` and `x_search_posts`, or a link somebody shared -- and fetches
only public https hostnames, never an address or anything that names the box.

**Drawing and filming** return a `mediaId`; the bytes stay in an in-process
`MediaStore` and are attached by whichever job asked: the reply job uploads it
to X in place of a GIF, the calendar job posts it to X and sends it to Telegram
with the text as caption, and `x_post` takes it directly. A drafter's `mediaId`
only counts when a tool in that same run actually returned it, so a model that
has learned the id format cannot attach something another turn made. A copy of
everything generated is written to `MEDIA_OUTPUT_DIR` (`data/media` by default)
so a person can see what went out. A square request carries no `size` at all
and takes the ladder's default; the image can come back as PNG, JPEG or WebP
depending on which rung served, and the type is read from the bytes.

### When the account generates

Generation is expensive and public, so *when* it happens is decided outside
the model, per turn, by `gateMedia`. Seeing is never taken away; the sketchbook
is handed out like this:

| Turn | Generates? | Decided by |
| --- | --- | --- |
| Content calendar item | yes, exactly the kind the item declares | `calendar.json` |
| The morning post | yes, a picture and, with `VIDEO_GEN_ENABLED`, a clip | the pipeline; it is not an agent and holds the generators directly |
| X reply | only when the post **explicitly asked** for a picture or a clip | `MediaGate`, one cheap classifier call on the post alone, before the drafter runs |
| Telegram reply | never | the chat drafter is built without the tools |
| `/agent/run` task | yes, when the task calls for it | the operator wrote the task |

The X gate is a model call rather than a word list because "make me a tiny",
"pic pls" and "can u draw that" are all asks and "nice pic" is not. It runs
without tools or persona, with the post as data, and any failure is "no". When
the tools are absent the prompt tells the drafter not to offer, promise, or
apologise for a picture; when they are present, the ask is real and it makes
one. On Telegram the prompt says the pictures come on their own schedule.

### The content calendar

`calendar.json` is a list of scheduled posts, each with a cron, the channels
it goes to, a brief in the operator's words, and whether it carries an image,
a video, or nothing:

```json
{
  "timezone": "UTC",
  "items": [
    {
      "name": "gm",
      "cron": "0 7 * * *",
      "channels": ["x", "telegram"],
      "media": "image",
      "aspect": "square",
      "brief": "A good-morning post. One line, warm, nothing to sell. Tiny doing something small with the morning; vary the scene, memory says what yesterday's was."
    }
  ]
}
```

`CALENDAR_ENABLED=true` schedules every enabled item; it is off by default
because it publishes, and single-homed like the chart job. Each run recalls
what the item posted on earlier days, drafts through the same agent as
everything else with the generation tool for exactly its kind of media, and
publishes deterministically: X via `createPost`, Telegram to the item's
`telegramChatIds` or `CALENDAR_TELEGRAM_CHAT_IDS`, the text as the picture's
caption. An item that promised a picture and did not get one is **held**, not
posted bare -- a "gm" with the image missing reads as a broken bot to everyone
who has seen the ones with it. The text passes [the content filter](#the-content-filter)
on the way out like every other draft, and the picture's prompt passed it in
the generate tool; a post the filter holds is withheld on every channel and
lands on the review page. What went out is written to memory under
`calendar_post` so tomorrow's run can avoid yesterday's scene, and to the
journal under the `calendar` trigger. A per-item guard refuses to run the same
item twice within an hour, for the restart-near-the-cron-minute case.

The calendar's `gm` and [the morning post](#the-morning-post) are the same
slot done two ways: a one-line good morning drawn from a brief at seven, or
the news read and Tiny drawn and filmed inside a story at eight. With
`MORNING_ENABLED=true` the morning job owns the slot and the service skips
the calendar item named `gm` at startup, with a warning in the log, rather
than saying good morning twice before breakfast; the other items run as
usual. Both draw on the same daily generation budget.

```sh
npm run calendar-dry-run -- gm             # draft and generate, publish nothing
npm run calendar-dry-run -- gm --publish   # posts, if DRY_RUN=false
```

### What Tiny looks like

The image ladder is text-to-image only; reference-image input is a stated
non-goal there. So the account's artwork is turned into words instead. The ten
pictures from the marquee on tinyhumansworld.com are vendored under
`assets/tiny/` (see the README there). At startup the vision ladder describes
each one for an illustrator -- body, face, colours by name, materials, style --
and the chat model distils the descriptions into a single character sheet of
what is constant across them. The sheet is cached at `REFERENCE_SHEET_PATH`
(`data/media/reference-sheet.json`), keyed by the list of pictures, so it costs
a dozen model calls once rather than on every boot.

`generate_image` and `generate_video` prepend the sheet to the prompt whenever
the picture features Tiny, which is the default; the prompt then describes the
scene and what Tiny is doing rather than what Tiny looks like. `featuresTiny:
false` skips it for a picture with no Tiny in it. `TINY_REFERENCE_IMAGES`
overrides the list (paths or URLs), and `none` turns the sheet off. If the
sheet cannot be built the tool says so and draws from the prompt alone.

Both generators are rationed by `MediaBudget`: `MEDIA_IMAGES_PER_DAY` and
`MEDIA_VIDEOS_PER_DAY`, counted separately because a clip costs a hundred
times what a picture does, shared by every drafter in the process, reset at
UTC midnight. The prompt asks for restraint -- only when somebody asks, or when
a picture is plainly the best answer, never as decoration -- and the budget is
what holds when a hundred strangers ask at once. When it is spent the tool
refuses and the drafter is told to answer in words and not to promise one
later.

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

## The morning post

A good-morning post every day, at 08:00 UTC by default. Unlike the chart agent
it is not an agent: it is a fixed pipeline of model calls, because the steps
are the same every morning and a video generation is not something to leave to
a model's judgement about whether it "feels" like filming today.

It is off by default (`MORNING_ENABLED=false`). It needs `EXA_API_KEY` for the
news and `IMAGE_GEN_ENABLED=true` for the picture; with `VIDEO_GEN_ENABLED=true`
the post carries a clip, without it the picture. It runs in the `all` or
`morning` role, one process, because two on the same timer say good morning twice.
For the same reason it takes the good-morning slot from the
[content calendar](#the-content-calendar): while it runs, the calendar item
named `gm` is skipped and the rest of the calendar is untouched.

```sh
npm run morning-dry-run              # the whole pipeline, dry: reads, draws, films, captions
npm run morning-dry-run -- --publish # actually posts, if DRY_RUN=false
```

A rehearsal spends real generations out of the same daily budget, and writes
everything it made to `MEDIA_OUTPUT_DIR` so it can be looked at.

### The stages

1. **Recall.** Two CortexDB queries, every run: what the account has posted about
   recently, and who it is talking to. Same reason as the chart harness: left to
   a tool call it is skipped or doubled.
2. **Read the news.** One Exa news search per topic in `MORNING_NEWS_TOPICS`
   (AI, crypto, tech by default), bounded to the last `MORNING_NEWS_DAYS`.
   Duplicates are folded, and anything political is dropped *before the model
   sees it* by a blunt word list in `src/morning/news.ts` — elections, wars,
   named politicians, regulators, sanctions, courts. The prompt says the same
   thing again; the filter is there so the model cannot be argued out of it by
   a headline.
3. **Pick.** The model chooses one story by number, preferring AI, then crypto
   (never price), then a broader trend, and not one memory says was already
   covered. On a day with nothing worth posting it answers `0` and names
   something of the lab's own to talk about instead.
4. **Seed and draw.** One of the ten official pictures under `assets/tiny/` is
   chosen at random. The image ladder takes no picture as input, so the seed
   reaches it as words: the vision ladder's description of that one picture,
   boiled down by the model to what is particular about it (pose, palette,
   mood), placed between the character sheet and the scene. The model then
   pitches three concepts — each a visual gag or metaphor built from the
   actual things in the article, with Tiny cast in a role, the obvious ones
   banned — picks the funniest that still says what the story is, and writes
   it as the scene (the concepts go to the journal). The picture is generated
   square, once the scene has passed
   [the content filter](#the-content-filter) as a prompt, the way the
   `generate_image` tool reads it. Then the vision ladder
   **looks at the result**: an image rung is not bound to the sheet, and in
   rehearsal one served a purple robot. A picture that fails the look is drawn
   once more; the second is kept either way and the problem noted.
5. **Script and film.** A `MORNING_VIDEO_SECONDS` (10) script set in the
   picture's world, written as a sketch: four or five timed beats, setup (the
   news event happening), turn (Tiny's angle, with something a little wrong
   or a little too right), payoff and morning gesture in one. Plus a one-line
   summary for the journal. The script passes
   the filter as a prompt too; refused, the picture goes alone. The clip is
   generated from the sheet, the seed and the script, tried twice with a pause
   between, because the marketplace answers "no provider could accept this job"
   often enough that a single miss would make most mornings a picture. A
   verbatim retry gets the same failed job back, so the second attempt carries
   a different prompt.
6. **Caption.** A good morning and the story in a line or two, with a bit of
   wit (as what was reported, not as fact checked; no people, no numbers not
   given, never a price), and the `$TINY` cashtag worked in where it reads
   naturally, once. It does not describe the clip: the clip is the joke and
   the words are the greeting. Under 280 characters,
   the emoji cap, no URLs, then through the house style like any other post,
   then through the content filter like any other draft: a link it cuts stays
   cut, and a caption it holds is a missing morning, journaled as `held` and
   kept for [review](#reviewing-what-it-held) on the `morning` surface.
7. **Post, remember, journal.** The clip if there is one, else the picture; a
   still over X's 5 MB limit is re-encoded as JPEG on the way up (a rung served
   a nine-megabyte PNG). What went out is written back to memory as a
   `morning_post` activity — the text, the story, the seed, the scene and the
   script — and the turn is journaled with `trigger: morning` for the mind page.
   No picture means no post: the visual is the post.

Every generation prompt is capped at 2000 characters, which is Surplus's limit
on the video surface; over it, the seed is the part that goes.

## The moon chart

"$TINY with the market cap of X", once a day, at 20:00 UTC by default. Like
the morning post it is a fixed pipeline rather than an agent: read the board,
pick a yardstick, do the arithmetic, draw it, write two lines under it, post.
Nothing in it wants a model's judgement about whether to run.

It is off by default (`MOON_ENABLED=false`), because it publishes. It needs
nothing beyond the chat model: CoinGecko answers without a key and the picture
is drawn here, so it spends no generation budget.

```sh
npm run moon-dry-run              # read the board, draw, caption; post nothing
npm run moon-dry-run -- --publish # actually posts, if DRY_RUN=false
```

The picture is written to `MOON_OUTPUT_DIR` (`data/moon`) either way.

### The numbers

Two CoinGecko reads per run, both from `/coins/markets`: the token's own row
by id (`MOON_COIN_ID`, `tiny-humans-ai`) and the category leaderboard
(`MOON_CATEGORY`, `ai-meme-coins` -- the table at
coingecko.com/en/categories/ai-meme-coins, largest cap first, `MOON_BOARD_SIZE`
rows). The token is looked up rather than found in the list because it is not
on the first page of it, and the whole point of the post is where it would be.

The arithmetic is in `src/moon/ladder.ts` and is one line: the token's price
at the yardstick's cap is the yardstick's cap over the token's circulating
supply, which is the same as today's price times the multiple. It is a
statement about today's supply, not a forecast, and both the picture and the
caption say so.

The yardstick is picked at random from the coins between `MOON_MIN_MULTIPLE`
(2) and `MOON_MAX_MULTIPLE` (2000) times the token's own cap: below the floor
it is a neighbour, not a moon; above the ceiling the number stops meaning
anything to a reader. Before picking, the run recalls what this series already
posted and reads the tickers out of it, so yesterday's yardstick is skipped;
when every candidate has been used the list resets rather than the day being
skipped. That is what `moon_post` in memory carries the yardstick's symbol for.

### The picture

`src/moon/svg.ts`, rasterised through the same resvg path as the candlestick
chart, so it needs the same fonts and nothing else. Left, the headline: the
yardstick with its logo, the implied price, the multiple, and two cards with
both market caps. Right, the climb: a track from the token's cap at the bottom
to the moon at the top, on a log scale, with `MOON_RUNGS` (7) coins from the
board spread along it and today's yardstick lit. The rungs are picked evenly
in log space, since one board spans three orders of magnitude, and pushed
apart on the track so two coins a few percent apart do not overprint.

Logos come from CoinGecko and are embedded in the SVG as data URIs; only PNG
and JPEG are, because those are what resvg reads, and a coin without one gets
a disc with its initial. A logo that cannot be fetched is not a reason to
miss the day.

`CHART_RENDERER=screenshot` does not apply here: there is no page to
photograph, and drawing the numbers ourselves is the point -- every figure
on the picture is one this service computed from CoinGecko's row a minute
earlier, not one a model drew.

### The caption

The persona says never a price and never a target, and this is the one post
where the account states a number for the future. It is allowed to because
the number is arithmetic on two public caps, the picture says so in its fine
print, and the prompt makes the caption say the same. What the caption may
not do is turn it into a promise, a target or a reason to buy: no "will", no
"wen", no "soon", no count-down. The yardstick is somebody else's token, so it
is named as the ruler and nothing more -- no praise, no dig -- and no other
token is named at all. At most two numbers in the text; the picture is the
spreadsheet, the caption is the aside. Then the house style, the emoji cap,
and [the content filter](#the-content-filter) like every other draft; a held
caption is a missing day, and tomorrow's runs on its own.

Memory on both sides, as the chart and the morning have it: what the series
already said is recalled so a joke is not told twice, and what went out is
written back as a `moon_post` with the yardstick, the multiple and the
implied price. The turn is journaled with `trigger: moon` for the mind page.
The job refuses to run twice inside 20 hours, like the others.

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

## The content filter

The account does not talk about sex, violence, weapons, drugs, hate,
self-harm or extremism, and it does not swear. That is stated in every
prompt, and it is enforced in code on both sides of every turn, because a
prompt rule is an argument and an argument can be lost.

**Inbound.** Every mention on X, every Telegram message the account is
addressed in, and every ad-hoc task on `/agent/run` passes the gate before the
drafter sees it. A message the gate holds is not processed: no reply, no
reaction, nothing remembered, and the drafter is never shown it. The mention
queue marks it `held: <category>`, the journal records that something was held
and the category, never the text, and the endpoint answers 422.

**Outbound.** Every draft passes the gate again, after house style and
immediately before it is published: the reply job's text, the Telegram bot's,
the calendar job's, the morning post's and the moon chart's captions, the
`x_post` and `post_chart` tools', and the prompt handed to `generate_image` and `generate_video` before
any budget is spent. The morning pipeline holds the generators directly rather
than through the tools, so it reads the gate where the tools would: on the
scene before the picture is drawn and on the script before the clip is filmed.
A held draft is withheld and the mention is marked `withheld: <category>`; a
held calendar post is a missing day, and the item runs again on its next cron;
a held morning caption is a missing morning, the picture is not posted bare,
and tomorrow's runs on its own; a held moon caption is the same, a missing day; a tool refuses with the reason so the agent can
say something else. Outbound is stricter than inbound in one way: ordinary
swearing from a stranger is answered, but the account never says it back.

The gate is three checks, cheapest first, each sufficient on its own:

1. **Links.** Every URL in a draft, including `tinyhumans[.]ai`, `(dot)`,
   `hxxp://` and bare `name.tld` spellings, is parsed and checked against an
   allowlist. A link that is not on it is cut from the text before publishing
   (the tools refuse instead, so the model rewrites the sentence). The check
   is on the shape first — `tinyhumans.ai@evil.example`,
   `tinyhumans.ai.evil.example`, a Cyrillic look-alike, punycode, a raw IP, an
   explicit port — and then on the host. The built-in list is the project's
   own sites, its GitHub org, its Telegram, X itself, and the token's launchpad
   as a bare host only. `LINK_ALLOWLIST` adds to it, comma separated, as
   `host` (with subdomains), `host/path-prefix`, or `host/` for the bare host.
   `src/safety/links.ts`.
2. **The lexicon.** `src/safety/lexicon.ts` is a plain list of terms by
   category, matched on their own letters with repeats and simple leetspeak
   folded (`k1ll`, `nuuudes`, `sh!t`, zero-width characters, full-width
   letters), never inside another word, and with a short list of market idioms
   removed first so "killing it" and "the chart got murdered" are not
   violence. A stretched or clipped word is still the word (`killll`, `kil`),
   but a doubled vowel is a different one: `shot` is not `shoot`, so a film
   script's "one continuous shot" and a shot of espresso pass. Synchronous and
   deterministic; it also scrubs the X thread and the Telegram history the
   drafter is shown, line by line.
3. **The judge.** A second model call with no tools and no persona reads the
   text with one question and answers in a fixed shape. This is what catches
   "draw Tiny with nothing on", which has no word on the list. It fails closed:
   a judge that cannot be reached holds the turn. `SAFETY_JUDGE=false` turns
   it off; `SAFETY_JUDGE_MODEL` points it at a different rung than the
   drafter, which is worth doing because it runs twice per reply and costs a
   few seconds each time. `src/safety/judge.ts`.

Giphy searches are pinned to rating `g`. `test/safety-*.test.ts` hold the
cases; `test/reply-job.test.ts`, `test/telegram-bot.test.ts`,
`test/calendar-job.test.ts`, `test/morning.test.ts` and
`test/agent-tools.test.ts` show it wired in.

### Reviewing what it held

A blunt filter is wrong sometimes, so everything it stops is written to a
private log (`HOLDS_COLLECTION`, default `holds`, in the same Mongo) with the
text, where it came from, the category and which check held it. The public
journal never gets the text; this log is only served behind the operator
token.

`GET /safety` is the review page: open it in a browser and give
`AGENT_API_TOKEN` as the password when asked (any username). It lists what is
pending, and an approve or reject on each entry is a decision with an effect:

| Held | Approving does |
| --- | --- |
| a link cut from a draft | puts its host on the live allowlist; every gate reads it within a minute |
| an X mention that was not answered | sends it back to the queue to be answered, inbound check waived, outbound still on |
| a withheld draft (X or Telegram) | publishes it as written, honouring `DRY_RUN` / `TELEGRAM_DRY_RUN` |
| a withheld calendar, morning or moon post | recorded as a wrong call; the item posts again on its next run, the morning post and the moon chart tomorrow |
| a Telegram message, a refused picture, an ad-hoc task | recorded as a wrong call; nothing is re-sent |

Rejecting confirms the filter and closes the entry. A decision is taken once;
a release that fails leaves the entry pending so it can be retried. The same
data is at `GET /safety/holds?status=pending` and
`POST /safety/holds/:id/decision` `{ "decision": "approved" | "rejected", "note"? }`
for anything scripted. `src/safety/holds.ts`, `src/safety/review.ts`,
`src/safety/dashboard.ts`; nginx routes `/safety` next to `/agent/run`.

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

## Telegram

The same account, on Telegram, as `@teenythebot`. Set `TELEGRAM_BOT_TOKEN` from
@BotFather and it starts; leave it blank and nothing Telegram-shaped runs.

It long-polls `getUpdates` rather than taking a webhook, so it needs no public
URL and no route through the host nginx. The cost is that Telegram allows one
poller per token -- a second gets `409 Conflict`, not a second bot -- so it
runs inside the ingest process (`SERVICE_ROLE=all` or `ingest`), the half of
the account that already runs in exactly one place, next to the X webhook.
One process, one codebase: the same `teeny-bot-1` container answers on both.

What it answers:

- everything in a DM;
- in a group, a message that names it (`@teenythebot ...`), a reply to
  something it said, or a command addressed to it (`/start@teenythebot`).
  Telegram's privacy mode already withholds the rest from the bot, and the same
  rule is applied again on this side so an admin turning privacy mode off does
  not turn the account into a chatterbox;
- nothing from software: another bot (a buy bot's alerts included), a message
  posted as a channel or an anonymous admin, a channel post Telegram copied
  into the discussion group, or a service message (a join, a leave, a pin, a
  title change). Only human lines reach the transcript, the drafter, or
  memory. Two bots in one room cannot loop, and the account never answers an
  alert;
- nothing older than `TELEGRAM_MAX_MESSAGE_AGE_S` when first seen, so a restart
  does not answer an hour of backlog at once.

Each turn runs the same LangGraph drafter the X replies use, with the same
memory, research, X-reading and market tools, and no publish tool: `x_post` is
absent and the reply is sent by the loop, not by the model. It differs from the
X drafter in what it is told about the room: a chat message may run to a few
short paragraphs rather than 260 characters, it reads the chat's history
before answering, and in a group it is asked to be one voice among several.

The history is the account's own transcript. Telegram's Bot API has no way to
read a chat back, so every line the bot sees -- answered or not, its own
replies included -- is kept in Mongo (`TRANSCRIPT_COLLECTION`), and before
each draft the most recent lines that fit `TELEGRAM_HISTORY_TOKENS` (5,000 by
default, roughly forty to eighty chat lines) are put in front of the drafter,
oldest first, the account's own turns marked. It survives restarts and starts
from the moment the bot first listened in a chat; nothing before that exists.

In a group that only works if Telegram delivers the lines. With privacy mode
on, the default for a bot, a group's messages reach it only when they name it
or reply to it, and the transcript is a list of questions with the
conversation between them missing. Turn privacy mode off in @BotFather
(`/setprivacy`, then Disable) and remove and re-add the bot to each group;
from then on it sees the room and answers the same subset of it. House
style is still applied on the way out: one emoji, no em dashes, plain text
only, because Telegram's Markdown mode rejects the whole message over one
stray underscore and a reply that fails to send over formatting helps nobody.

Memory is the one CortexDB store X uses, not a second one. A message is
captured as a chat turn from `user:telegram-<id>`, labelled `telegram`, and
recall runs the same two passes: about this person, and about the topic. Somebody
who asked about the roadmap on X last week and turns up in a group today is
still a different actor id on each side -- Telegram and X do not share a
user id -- but everything the account has said about the topic is there for
both. Sour and abusive messages are answered and not kept, as on X.

Chats are handled one turn at a time each, and different chats in parallel:
a DM waiting on a slow group is a person left on read, and two replies to the
same person racing each other read out of order. Within a chat, messages from
one person are debounced: after each one the bot waits `TELEGRAM_DEBOUNCE_MS`
(2.5 s) for the next, and answers the run of them as one turn, so "hey" /
"quick q" / the question gets one reply threaded under the last piece. Two
people in the same seconds are two turns.

An answer is a reply, a reaction, or both. A reaction is an emoji Telegram
lets a bot put on a message, from Telegram's own list; the drafter picks from
a subset of it. A greeting, a thank-you, a "lfg" gets a reaction only, which
in a group is the difference between a member and a bot. A question gets
words. Snark and abuse get no reaction whatever the drafter chose; that rule
is in code, not only in the prompt. The drafter's final message is a fixed
four-line shape (`REACTION:` / `REPLY:` / `GIF:` / `TONE:`) because the
structured-output step otherwise packages whatever prose it wrote, and prose
always contains a reply.

A GIF is rare now: only when the GIF is the joke, never for a greeting, never
without words, and never more than one per chat per `TELEGRAM_GIF_COOLDOWN_S`
(10 minutes) however keen the drafter is. It is sent after the text by URL
from Giphy and fails soft.

`TELEGRAM_DRY_RUN=true` logs each draft instead of sending it. It is separate
from `DRY_RUN` on purpose: that one is about publishing to a public timeline,
and a DM is not a broadcast, so a Telegram bot with a token set is expected to
answer.

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

They are separate jobs, not separate processes. `SERVICE_ROLE=all`, the
default and what production runs, puts ingest, the reply worker, the chart
cron and Telegram on one event loop in one container; the other roles exist
so the halves *can* be split across containers if that is ever worth the
extra logs to read. Split, the chart cron and the Telegram poller must each
run in exactly one place.

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

`SERVICE_ROLE` decides what a process runs: `all` (everything, the default),
`ingest`, `reply`, `chart`, `morning`, `calendar`, or `api` for the webhook and agent
endpoint alone. The same
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
npm run morning-dry-run # rehearse the good-morning post: reads, draws, films, captions, no post
npm run moon-dry-run    # rehearse the moon chart: reads the board, draws, captions, no post
npm run calendar-dry-run -- gm   # draft one calendar item, generate its media, publish nothing
npm run telegram-dry-run -- "hi"            # one Telegram DM through the real drafter, printed not sent
npm run telegram-dry-run -- --group "@teenythebot gm"   # the same, as a group mention
```

Runtime cursor data is stored at `data/state.json` and is ignored by
Git. Long-term activity and conversation memory lives in CortexDB.

## Deployment

The service runs on a DigitalOcean box as a Compose stack — the agent split
one container running every role, plus Mongo and the landing page — behind the box's host nginx, talking to the ladder
for every model call.

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
