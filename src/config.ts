import { z } from 'zod'
import { DEFAULT_REFERENCE_IMAGES } from './media/reference.js'

const booleanFromString = z
  .enum(['true', 'false'])
  .default('true')
  .transform((value) => value === 'true')

/**
 * A value that may be left blank.
 *
 * `.optional()` alone is not enough: it permits the key to be absent, but a
 * filled-in .env spells "absent" as `KEY=`, and an empty string is a string,
 * which then fails the format check and takes the process down at boot. Every
 * optional value with a format has to accept the empty spelling too.
 */
function blankable<T extends z.ZodType>(schema: T) {
  return z.preprocess((value) => (value === '' ? undefined : value), schema.optional())
}

/** For switches whose safe position is off rather than on. */
const offByDefault = z
  .enum(['true', 'false'])
  .default('false')
  .transform((value) => value === 'true')

/** Days of the posting rota, comma separated: `1,4`. */
const rotaSlots = (fallback: string) =>
  z
    .string()
    .default(fallback)
    .transform((value) => value.split(',').map((entry) => Number(entry.trim())).filter((entry) => Number.isInteger(entry) && entry >= 0))

const schema = z.object({
  PORT: z.coerce.number().int().min(1).max(65535).default(3001),
  LOG_LEVEL: z.string().default('info'),
  DRY_RUN: booleanFromString,
  REPLY_CRON: z.string().default('*/5 * * * *'),
  MAX_REPLIES_PER_RUN: z.coerce.number().int().min(1).max(100).default(5),
  X_BOT_USERNAME: z.string().min(1),
  X_API_KEY: z.string().min(1),
  X_API_SECRET: z.string().min(1),
  X_ACCESS_TOKEN: z.string().min(1),
  X_ACCESS_TOKEN_SECRET: z.string().min(1),
  X_BEARER_TOKEN: z.string().optional(),
  // The read cache in front of X. Every post a read returns is billed, so a
  // post once seen is kept for X_CACHE_POST_TTL_DAYS and served from Mongo,
  // and a search, conversation or trend list is kept for
  // X_CACHE_SEARCH_TTL_MINUTES so a job asking the same thing twice in one
  // run pays once. Off, every read goes to X.
  X_CACHE_ENABLED: z.coerce.boolean().default(true),
  X_CACHE_POST_TTL_DAYS: z.coerce.number().min(0).default(30),
  X_CACHE_SEARCH_TTL_MINUTES: z.coerce.number().min(0).default(10),
  CORTEX_API_URL: z.url().default('https://api-v1.cortexdb.ai'),
  CORTEX_ACTOR: z.string().min(1).default('agent:teeny'),
  CORTEX_SCOPE: z.string().min(1).default('agent:teeny'),
  CORTEX_TOKEN: z.string().min(1),
  LLM_BASE_URL: z.url().default('https://api.openai.com/v1'),
  LLM_API_KEY: z.string().min(1),
  LLM_MODEL: z.string().min(1),
  EXA_API_KEY: z.string().min(1).optional(),
  AGENT_MODE: z.enum(['langgraph', 'simple']).default('langgraph'),
  AGENT_MAX_STEPS: z.coerce.number().int().min(1).max(100).default(24),
  AGENT_API_TOKEN: z.string().min(16).optional(),
  // A copy of every verified X delivery is POSTed here, body and signature
  // untouched, after X has been answered. Unset, nothing is forwarded.
  WEBHOOK_FORWARD_URL: z.url().optional(),
  WEBHOOK_FORWARD_TOKEN: z.string().min(16).optional(),
  SOUL_PATH: z.string().default('SOUL.md'),
  CONTEXT_PATH: z.string().default('CONTEXT.md'),
  SKILLS_DIR: z.string().default('skills'),
  MAX_EMOJI_PER_POST: z.coerce.number().int().min(0).max(10).default(1),
  // --- the content filter ---------------------------------------------------
  // Every message in and every draft out passes a lexicon and a link allowlist
  // regardless; these dials control the model judge that runs after them for
  // anything phrased around the words. On by default and on the chat model,
  // because the filter is the one part of the account that should not have an
  // off switch a busy operator forgets to flip back.
  SAFETY_JUDGE: booleanFromString,
  SAFETY_JUDGE_MODEL: blankable(z.string().min(1)),
  // --- the ranker's reader ---------------------------------------------------
  // A separate model call that reads a draft the way X's For You ranker would
  // and hands back the arithmetic and the fixes. Offered to the agents as
  // x_critique, run by the morning pipeline on its script and caption, and
  // served at POST /critique. Advisory, never a hold. On by default, on the
  // ladder's fast rung: a read is a dozen numbers and three lines, and the
  // reasoning rung took a minute over it, which a caption can wait for and an
  // agent turn cannot.
  ALGO_CRITIC: booleanFromString,
  ALGO_CRITIC_MODEL: z.preprocess((value) => (value === '' ? undefined : value), z.string().min(1).default('flash')),
  // Hosts the account may link to beyond the built-in list, comma separated,
  // as `host`, `host/path-prefix`, or `host/` for the bare host only.
  LINK_ALLOWLIST: z
    .string()
    .default('')
    .transform((value) => value.split(',').map((entry) => entry.trim()).filter(Boolean)),
  GIPHY_API_KEY: z.string().min(1).optional(),
  GIPHY_CHANNEL: z.string().min(1).default('TinyHumansAI'),
  // X bills per post retrieved, so both of these are cost dials.
  MENTION_LOOKBACK_MINUTES: z.coerce.number().int().min(1).max(10_080).default(15),
  X_SEARCH_MAX_RESULTS: z.coerce.number().int().min(10).max(100).default(10),
  // `all` runs every role in one process. Split them to scale or restart the
  // reply side without pausing ingestion, which is the half with a deadline.
  SERVICE_ROLE: z.enum(['all', 'ingest', 'reply', 'api', 'chart', 'morning', 'moon', 'calendar', 'narrative', 'article', 'quote', 'lore']).default('all'),
  INGEST_CRON: z.string().default('*/5 * * * *'),
  // The X Activity API's stream, `GET /2/activity/stream`, read next to the
  // webhook. On because the webhook stopped delivering `post.*` events in
  // 2026-08 (an X-side fault, devcommunity 273644) while the stream still
  // does, within seconds. Needs X_BEARER_TOKEN; without one it is skipped.
  ACTIVITY_STREAM_ENABLED: z.coerce.boolean().default(true),
  // `worker` drains the queue continuously, which is what makes a webhook
  // delivery turn into a reply in seconds rather than by the next tick.
  // `cron` keeps the old timer for a quiet account that would rather poll.
  REPLY_MODE: z.enum(['worker', 'cron']).default('worker'),
  // How long the worker waits after finding an empty queue. One indexed query
  // per tick, so this is cheap; it is latency, not load.
  REPLY_IDLE_MS: z.coerce.number().int().min(250).max(60_000).default(2_000),
  // The gap between two published replies, picked at random in this range. A
  // giveaway had the account answering a hundred people in an hour and X
  // throttled its reach for it. The range rather than a single number because
  // a reply exactly every sixty seconds is its own signature: no one typing by
  // hand is that regular. Set the minimum to zero to disable pacing entirely.
  REPLY_MIN_INTERVAL_MS: z.coerce.number().int().min(0).max(3_600_000).default(30_000),
  REPLY_MAX_INTERVAL_MS: z.coerce.number().int().min(0).max(3_600_000).default(60_000),
  MONGO_URI: z.string().min(1).default('mongodb://mongo:27017'),
  MONGO_DATABASE: z.string().min(1).default('teeny'),
  MENTIONS_COLLECTION: z.string().min(1).default('mentions'),
  // --- the brain, read-only ---
  // The brain's checkout (README.md, "The brain"): the definition under
  // company/<slug>/, the host's record under companies/<slug>/. The bot reads
  // it to fold the agents' token spend into the ledger and to serve the
  // roster and the latest runs at /brain. Locally that is the submodule; on
  // the box it is BRAIN_DIR mounted read-only. Blank it to serve neither.
  BRAIN_DATA_DIR: blankable(z.string().min(1)).default('brain'),
  BRAIN_COMPANY: z.string().min(1).default('teeny'),
  BRAIN_USAGE_CRON: z.string().default('* * * * *'),
  // --- the content calendar's cadence ---
  // The least time between two of the account's own posts, whichever job
  // writes them; a job whose slot comes sooner skips it. The tolerance is
  // for the minutes a job spends drawing and filming after its cron fires.
  // Three posts a day, eight hours apart: 07:00, 15:00, 23:00 UTC. The gap
  // is an hour short of that so a slow job (the article, a filmed morning)
  // that posts late does not cost the next slot its turn.
  POST_MIN_GAP_HOURS: z.coerce.number().min(0).max(48).default(7),
  POST_GAP_TOLERANCE_MINUTES: z.coerce.number().min(0).max(120).default(20),
  // The heat job: reads the last day's own posts' counters on this cron and
  // holds the timeline for one that is taking off. Hot is at least this many
  // impressions an hour and this many times the account's usual peak; the
  // hold is one slot (POST_MIN_GAP_HOURS), extended while it stays hot, up
  // to HOT_MAX_HOLD_SLOTS from when it first ran hot.
  HEAT_ENABLED: z.coerce.boolean().default(true),
  HEAT_CRON: z.string().default('*/20 * * * *'),
  HOT_IMPRESSIONS_PER_HOUR: z.coerce.number().min(1).default(500),
  HOT_BASELINE_MULTIPLE: z.coerce.number().min(1).default(3),
  HOT_MAX_HOLD_SLOTS: z.coerce.number().int().min(1).max(6).default(2),
  // The reply budget: answers under each own post, and answers out in the
  // world per own post published in the last day. A hot post's held slots
  // add both allowances to its own thread. Every mention taken is liked
  // whether or not it is answered; likes are not posts and are not budgeted.
  REPLY_AUTO_PER_POST: z.coerce.number().int().min(0).max(100).default(10),
  REPLY_SMART_PER_POST: z.coerce.number().int().min(0).max(100).default(10),
  REPLY_LIKE_MENTIONS: z.coerce.boolean().default(true),
  // Where the content filter keeps what it held, for review at /safety.
  HOLDS_COLLECTION: z.string().min(1).default('holds'),
  // How much of the thread is fetched before drafting. Zero disables it, and
  // the reply then sees only the post that mentioned the account. Every post
  // retrieved is billed and X's floor on a search is ten, so this is the most
  // expensive part of a reply: at five replies a run it is the difference
  // between fifty posts and none.
  // The account's own token contracts, comma separated. Matched against every
  // post before drafting, because a model will not reliably match a contract
  // address against a long persona document -- and getting this wrong means
  // calling your own token somebody else's shill, in public.
  OWN_TOKEN_ADDRESSES: z
    .string()
    .default('')
    .transform((value) => value.split(',').map((entry) => entry.trim()).filter(Boolean)),
  THREAD_CONTEXT_MAX_POSTS: z.coerce.number().int().min(0).max(100).default(10),
  // Handles whose statements the account takes as fact: the lab and the people
  // who run it. Matched against the author and the thread before drafting, so
  // "Teeny buys $TINY" from the team is known rather than "reportedly".
  TRUSTED_HANDLES: z
    .string()
    .default('TinyHumansAISol,senamakel,enamakel,tinyhumansai')
    .transform((value) => value.split(',').map((entry) => entry.trim()).filter(Boolean)),
  // The names the account goes by besides its handle, matched as whole words
  // so a post about "Teeny" is recognised as a post about the account.
  SELF_NAMES: z
    .string()
    .default('Teeny')
    .transform((value) => value.split(',').map((entry) => entry.trim()).filter(Boolean)),

  // --- the chart agent -----------------------------------------------------
  // A second agent, on its own timer: it reads the token's own chart once a day
  // and posts it. Off by default, because it publishes.
  CHART_ENABLED: offByDefault,
  // Its day is CHART_ROTA_SLOT in the afternoon slot (see the rota). A
  // post lands or it does not; there is no catch-up.
  // The token the chart agent looks at. Bound in configuration rather than
  // chosen per run: this agent reads posts, and an agent that can be told which
  // contract to analyse can be told to analyse somebody else's.
  CHART_TOKEN_ADDRESS: blankable(z.string().min(26)),
  CHART_TOKEN_SYMBOL: z.string().min(1).default('TINY'),
  CHART_CHAIN: z.string().min(1).default('solana'),
  CHART_TIMEFRAME: z.enum(['5m', '15m', '1h', '4h', '12h', '1d', '1w']).default('1h'),
  CHART_BARS: z.coerce.number().int().min(20).max(500).default(120),
  // `native` draws the chart here from candles this service fetched. `screenshot`
  // drives a headless browser at a page that draws one; see ScreenshotChartRenderer
  // for why that is not the default.
  CHART_RENDERER: z.enum(['native', 'screenshot']).default('native'),
  CHART_URL_TEMPLATE: blankable(z.string().min(1)),
  CHART_BROWSER_CHANNEL: blankable(z.string().min(1)),
  CHART_BROWSER_PATH: blankable(z.string().min(1)),
  CHART_OUTPUT_DIR: z.string().default('data/charts'),
  // What the community is searched for before the daily post. Blank disables
  // the pulse tool, and the agent is then writing to holders without having
  // read anything they said today. One X search per run, floor of ten posts.
  CHART_COMMUNITY_QUERY: z.string().default('$TINY -is:retweet'),
  // Defaults to the account's own handle at startup.
  CHART_WATERMARK: blankable(z.string().min(1)),
  // A ranked-trader source. Unset, the KOL tool returns nothing and the prompt
  // forbids claiming any -- which is the correct behaviour, not a degradation.
  GMGN_BASE_URL: blankable(z.url()),
  SOLANA_RPC_URL: blankable(z.url()),
  // Wallets worth naming when they show up in the flow, as `address:label`
  // pairs, comma separated. The label is for the agent's reasoning only; the
  // prompt forbids putting an address in a post.
  KOL_WALLETS: z
    .string()
    .default('')
    .transform((value) =>
      value
        .split(',')
        .map((entry) => entry.trim())
        .filter(Boolean)
        .flatMap((entry) => {
          const [wallet, ...rest] = entry.split(':')
          return wallet ? [{ wallet: wallet.trim(), label: rest.join(':').trim() || 'watchlist' }] : []
        }),
    ),

  // --- the morning post ----------------------------------------------------
  // A good-morning post every day: the news read, one non-political story
  // chosen, Tiny drawn inside it from one of the official pictures, a short
  // clip filmed from that, and the lot captioned and posted. Off by default,
  // because it publishes and because it spends an image and a video a day.
  // Needs EXA_API_KEY and IMAGE_GEN_ENABLED; VIDEO_GEN_ENABLED makes it a clip.
  MORNING_ENABLED: offByDefault,
  // 07:00 UTC, the morning slot, every day: morning in Europe, the small
  // hours in New York, afternoon in Asia. A daily post lands or it does
  // not; there is no catch-up.
  MORNING_CRON: z.string().default('0 7 * * *'),
  // What is searched for, comma separated. Each is one Exa query over the
  // news index, bounded to MORNING_NEWS_DAYS. Politics is filtered out of
  // the results before the model sees them, whatever the query.
  MORNING_NEWS_TOPICS: z
    .string()
    .default('artificial intelligence news,crypto and web3 news,technology trends')
    .transform((value) => value.split(',').map((entry) => entry.trim()).filter(Boolean)),
  MORNING_NEWS_DAYS: z.coerce.number().int().min(1).max(7).default(2),
  MORNING_STORIES_PER_TOPIC: z.coerce.number().int().min(1).max(20).default(8),
  // The timeline next to the news: X's recent search over a few corners (AI
  // memes first, then AI, then crypto builders), and the trends list where
  // the tier has it. Each corner may bring this many posts (the meme corner
  // twice that); X bills every post read and floors a page at 10. Twenty is
  // about eighty posts a morning, of which a third survive the quiet-post
  // floor; 0 turns the timeline off.
  MORNING_X_POSTS_PER_QUERY: z.coerce.number().int().min(0).max(100).default(20),
  MORNING_X_HOURS: z.coerce.number().int().min(1).max(168).default(24),
  // `label=query;label=query` to replace the built-in corners; X search syntax.
  MORNING_X_QUERIES: z.string().default(''),
  // Whose trends: a Yahoo WOEID. The United States by default; 1 is the world,
  // which is mostly K-pop and cricket by the time it is morning in UTC.
  MORNING_X_TRENDS_WOEID: z.coerce.number().int().min(0).default(23_424_977),
  // How long a clip is scripted for. Passed to the generator as a hint;
  // Sora rounds it to 4, 8 or 12, and a ladder rung may render its own.
  // The morning from the lab: the chosen story seeds a backrooms run, the
  // room's thought is what the picture is drawn from, and the good morning
  // is written from it with the soul, the lore and the last posts around it.
  // Off, the caption prompt writes the morning from the headline as before.
  MORNING_BACKROOMS: offByDefault,
  MORNING_BACKROOMS_TURNS: z.coerce.number().int().min(2).max(60).default(10),
  MORNING_VIDEO_SECONDS: z.coerce.number().int().min(4).max(20).default(10),

  // --- the posting rota ----------------------------------------------------
  // The calendar: three slots a day, eight hours apart, and a three-day rota
  // that says which job has each slot on each day -- nine posts a cycle.
  // Days are counted in UTC from the anchor, which is slot 0.
  //
  //   07:00 the morning post, every day (MORNING_CRON)
  //   15:00 the afternoon slot (AFTERNOON_CRON): the chart (slot 0), the
  //         moon chart (slot 1), the lore post on the rest
  //   23:00 the evening slot (EVENING_CRON): the article (slot 1), the
  //         serial (slot 2), the quote post on the rest
  //
  // Each slot is one cron that runs the day's owner. A slot past the rota's
  // length never posts, and a job that is off hands its days to the slot's
  // fallback: the lore in the afternoon, the quote in the evening.
  POST_ROTA_DAYS: z.coerce.number().int().min(1).max(14).default(3),
  POST_ROTA_ANCHOR: z.string().regex(/^\d{4}-\d{2}-\d{2}$/).default('2026-09-14'),
  AFTERNOON_CRON: z.string().default('0 15 * * *'),
  CHART_ROTA_SLOT: z.coerce.number().int().min(0).default(0),
  MOON_ROTA_SLOT: z.coerce.number().int().min(0).default(1),
  EVENING_CRON: z.string().default('0 23 * * *'),
  ARTICLE_ROTA_SLOTS: rotaSlots('1'),
  NARRATIVE_ROTA_SLOTS: rotaSlots('2'),

  // --- the moon chart ------------------------------------------------------
  // "$TINY with the market cap of X", once a day: the token's row and the AI
  // meme coin leaderboard read from CoinGecko, one coin above it picked as the
  // yardstick, the implied price drawn as a picture, two lines under it. Off
  // by default, because it publishes; single-homed like the chart.
  MOON_ENABLED: offByDefault,
  // Its day is MOON_ROTA_SLOT in the afternoon slot (see the rota).
  // The account's own token, by CoinGecko id, and the category the yardsticks
  // come from: coingecko.com/en/categories/<MOON_CATEGORY>.
  MOON_COIN_ID: z.string().min(1).default('tiny-humans-ai'),
  MOON_CATEGORY: z.string().min(1).default('ai-meme-coins'),
  MOON_BOARD_NAME: z.string().min(1).default('AI meme coins'),
  // The whole board is read, paged at 250: the token has to be found on it,
  // and the yardsticks are the coins above it there.
  MOON_BOARD_SIZE: z.coerce.number().int().min(5).max(1_000).default(500),
  MOON_RUNGS: z.coerce.number().int().min(1).max(12).default(7),
  // CoinGecko ids never used as the yardstick, comma separated: coins the
  // "AI Meme" tag is generous about. Tutorial is a BNB four.meme launch that
  // carries the tag; it is not what anybody means by an AI meme.
  MOON_EXCLUDE_IDS: z
    .string()
    .default('tutorial')
    .transform((value) => value.split(',').map((entry) => entry.trim()).filter(Boolean)),
  // The band a yardstick is picked from, as a multiple of the token's own
  // cap. Under the floor it is a neighbour, not a moon; over the ceiling the
  // number stops meaning anything to a reader.
  MOON_MIN_MULTIPLE: z.coerce.number().min(1).default(2),
  MOON_MAX_MULTIPLE: z.coerce.number().min(1).default(2_000),
  MOON_OUTPUT_DIR: z.string().default('data/moon'),

  // --- the lore post -------------------------------------------------------
  // A page from the lab, on the afternoon slot's days the charts do not
  // have: something from the wire seeds a backrooms run
  // inside the lore, the editor keeps one thought, and it is written up in
  // teeny's voice with the soul, the lore and the last posts around it.
  // Text only; the serial and the morning spend the media budget. Off by
  // default, because it publishes.
  LORE_ENABLED: offByDefault,
  // Where the seed comes from, tried in a rotating order until one answers:
  // news (Exa, then Google News), forum (reddit/4chan), memory (a recall).
  LORE_SEEDS: z
    .string()
    .default('news,forum')
    .transform((value) => value.split(',').map((entry) => entry.trim()).filter(Boolean))
    .pipe(z.array(z.enum(['news', 'forum', 'memory'])).min(1)),
  // What the wire is asked for; one is drawn a day.
  LORE_TOPICS: z
    .string()
    .default('ai agents,robots,artificial intelligence,open source ai,memecoins')
    .transform((value) => value.split(',').map((entry) => entry.trim()).filter(Boolean)),
  LORE_BACKROOMS_TURNS: z.coerce.number().int().min(2).max(60).default(10),

  // --- the quote post ------------------------------------------------------
  // Somebody else's post lifted onto the timeline with a line
  // over it: a holder bullish on $TINY, somebody posting the ticker or the
  // contract with something to say, or somebody who talked to the account.
  // Two X searches a run, a filter, one model call, a quote post. Off by
  // default, because it publishes; single-homed like the chart.
  QUOTE_ENABLED: offByDefault,
  // It has the evening slot's days the article and the serial do not:
  // 23:00 UTC, evening in the Americas, where the bulls are.
  // The ticker search, X syntax. People paste the contract as
  // `solana:<address>`, and X has no `solana:` operator, so the address is
  // a quoted phrase. Retweets and the account's own posts are cut by the
  // pipeline; a second search reads the account's mentions.
  QUOTE_QUERY: z.string().min(1).default('$TINY OR "2AF7CqwieUjUPALL7icuZtL3X7wENdjUjGBMmfV2pump"'),
  // How far back "recent" reaches, and how many posts each search reads
  // (X floors a page at 10 and bills every post).
  QUOTE_HOURS: z.coerce.number().int().min(1).max(168).default(48),
  QUOTE_POSTS_PER_SEARCH: z.coerce.number().int().min(10).max(100).default(50),
  // An author quoted inside this window is not quoted again, so a stranger
  // posting the ticker every hour does not get the timeline every night.
  QUOTE_AUTHOR_COOLDOWN_DAYS: z.coerce.number().int().min(0).max(90).default(7),
  // What has been quoted, on the data volume. A post is quoted once, ever.
  QUOTE_LOG_PATH: z.string().default('data/quotes.json'),

  // --- the serial ----------------------------------------------------------
  // Teeny's own story, an episode at a time: a writers' room between a hot
  // model in teeny's voice and a cooler producer, cut into a ten-second
  // clip and a caption in teeny's words, continued from the last episode.
  // Off by default, because it publishes. Needs IMAGE_GEN_ENABLED; with
  // VIDEO_GEN_ENABLED the episode is a clip. Shares the media budget with
  // the morning post, so it runs in the evening when the morning has had
  // its clip.
  NARRATIVE_ENABLED: offByDefault,
  // Its days are NARRATIVE_ROTA_SLOTS in the evening slot, once a cycle:
  // a serial needs regularity more than frequency, and the audience has to
  // remember the last episode.
  NARRATIVE_VOICE_PATH: z.string().default('narrative/VOICE.md'),
  NARRATIVE_LORE_PATH: z.string().default('narrative/LORE.md'),
  NARRATIVE_STATE_PATH: z.string().default('data/narrative-state.json'),
  // Where a visitor's likeness, written from a photograph found on the web, is kept between episodes.
  NARRATIVE_LIKENESS_DIR: z.string().default('data/likeness'),
  // How many writers' rooms run per episode and how many exchanges each has.
  NARRATIVE_BRANCHES: z.coerce.number().int().min(1).max(6).default(3),
  NARRATIVE_TURNS: z.coerce.number().int().min(2).max(8).default(6),
  NARRATIVE_VIDEO_SECONDS: z.coerce.number().int().min(4).max(20).default(10),
  // The heat of the two writers. Teeny is hot, because a cold model never
  // says the absurd thing; the producer is cooler, because somebody has to
  // hold the camera.
  NARRATIVE_WILD_TEMPERATURE: z.coerce.number().min(0).max(2).default(1),
  NARRATIVE_STRAIGHT_TEMPERATURE: z.coerce.number().min(0).max(2).default(0.6),
  // Telegram chats the episode is also sent to, comma separated.
  NARRATIVE_TELEGRAM_CHAT_IDS: z
    .string()
    .default('')
    .transform((value) => value.split(',').map((entry) => entry.trim()).filter(Boolean)),

  // --- the backrooms -------------------------------------------------------
  // A room where tool-using agents talk about whatever came through the
  // wire, from big and different contexts. Exploration only: runs land in
  // BACKROOMS_DIR as a notebook and nothing posts.
  BACKROOMS_DIR: z.string().default('data/backrooms'),
  // The editor's standing instructions: the prompt that turns a transcript into one thought.
  BACKROOMS_EDITOR_PATH: z.string().default('narrative/EDITOR.md'),
  // Tokens of context each agent starts with, and how many turns the room runs.
  BACKROOMS_BUDGET_TOKENS: z.coerce.number().int().min(1_000).max(30_000).default(9_000),
  BACKROOMS_TURNS: z.coerce.number().int().min(2).max(60).default(12),
  BACKROOMS_AGENTS: z
    .string()
    .default('teeny,swarm')
    .transform((value) => value.split(',').map((entry) => entry.trim()).filter(Boolean)),
  // How many things an agent may look up before it has to say something.
  BACKROOMS_TOOL_CALLS: z.coerce.number().int().min(0).max(8).default(3),
  // Where the craziness comes from. Read-only public endpoints, no accounts.
  REDDIT_SUBS: z
    .string()
    .default('singularity,artificial,ControlProblem,LocalLLaMA,conspiracy')
    .transform((value) => value.split(',').map((entry) => entry.trim()).filter(Boolean)),
  FOURCHAN_BOARDS: z
    .string()
    .default('g,x,sci,biz')
    .transform((value) => value.split(',').map((entry) => entry.trim()).filter(Boolean)),

  // --- the article ---------------------------------------------------------
  // A long piece of the serial's lore, once a cycle, published as an X
  // Article. The backrooms find the idea; the piece is planned, written,
  // proof-read and drawn, and the whole run is saved under ARTICLES_DIR.
  // Off by default, and a rehearsal even when on: nothing reaches X until
  // ARTICLE_PUBLISH is set, so the job can run on the box and its output be
  // read on the site before a single article goes out. X allows articles on
  // a Premium+ account only.
  ARTICLE_ENABLED: offByDefault,
  ARTICLE_PUBLISH: offByDefault,
  // Its days are ARTICLE_ROTA_SLOTS in the evening slot, once a cycle.
  ARTICLES_DIR: z.string().default('data/articles'),
  // How many rooms are opened on the same seed; the best-scored one is written up.
  ARTICLE_ROOMS: z.coerce.number().int().min(1).max(4).default(2),
  ARTICLE_SECTIONS_MAX: z.coerce.number().int().min(3).max(5).default(4),
  ARTICLE_WORDS: z.coerce.number().int().min(600).max(3_000).default(1_500),
  // What the room is seeded with: a story from the wire on one of these.
  ARTICLE_NEWS_TOPICS: z
    .string()
    .default('artificial intelligence news,robots and automation,crypto and web3 news')
    .transform((value) => value.split(',').map((entry) => entry.trim()).filter(Boolean)),
  // The proof-reader's checklist.
  ARTICLE_HUMANIZER_PATH: z.string().default('skills/humanizer/SKILL.md'),
  ARTICLE_WRITER_TEMPERATURE: z.coerce.number().min(0).max(2).default(0.8),
  // Refuses to publish again inside this window, on disk, so a restart at the cron minute cannot post twice.
  ARTICLE_MIN_INTERVAL_HOURS: z.coerce.number().min(1).max(168).default(48),

  // --- the content calendar ------------------------------------------------
  // Scheduled posts from a JSON file: a "gm" every morning, a Friday post, a
  // Sunday clip. This, with the morning post above, is where the account
  // generates pictures without being asked, so it is off by default, because
  // it publishes, and single-homed like the chart job. Each item names its
  // own cron and channels; see calendar.json and src/calendar/calendar.ts.
  // With MORNING_ENABLED the "gm" item is skipped: the morning job has it.
  CALENDAR_ENABLED: offByDefault,
  CALENDAR_PATH: z.string().default('calendar.json'),
  // Telegram chats an item posts to when it names none of its own.
  CALENDAR_TELEGRAM_CHAT_IDS: z
    .string()
    .default('')
    .transform((value) => value.split(',').map((entry) => entry.trim()).filter(Boolean)),

  // --- telegram ------------------------------------------------------------
  // The same account on Telegram: DMs, and groups it is named in. Set the
  // token and it runs; leave it blank and nothing Telegram-shaped starts. It
  // runs inside the ingest process, next to the webhook: long polling means
  // exactly one process may hold the token (a second gets 409s, not a second
  // bot), and ingest is already the half that runs in exactly one place.
  TELEGRAM_BOT_TOKEN: blankable(
    z.string().regex(/^\d+:[A-Za-z0-9_-]{30,}$/, 'must be a Telegram bot token'),
  ),
  // Separate from DRY_RUN, which is about publishing to a public timeline. A
  // DM is not a broadcast, and a Telegram bot that never answers is a bot
  // nobody can tell is working. On, drafts are logged and not sent.
  TELEGRAM_DRY_RUN: offByDefault,
  // A message older than this when it is first seen is skipped, so a restart
  // does not answer an hour of backlog all at once.
  TELEGRAM_MAX_MESSAGE_AGE_S: z.coerce.number().int().min(10).max(86_400).default(300),
  TELEGRAM_POLL_TIMEOUT_S: z.coerce.number().int().min(1).max(50).default(30),
  // How much of a chat's recent history the drafter is shown before it
  // answers, in tokens, read back from the transcript it keeps in Mongo. Five
  // thousand is roughly forty to eighty chat lines: enough to answer inside a
  // conversation rather than beside it, cheap next to the persona documents.
  TELEGRAM_HISTORY_TOKENS: z.coerce.number().int().min(0).max(50_000).default(5_000),
  TRANSCRIPT_COLLECTION: z.string().min(1).default('telegram_transcript'),
  // How long to wait for the same person to finish typing before answering.
  // "hey" / "quick q" / the question, sent as three messages, is one thought.
  TELEGRAM_DEBOUNCE_MS: z.coerce.number().int().min(0).max(30_000).default(2_500),
  // The least time between two GIFs in one chat, whatever the drafter wants.
  TELEGRAM_GIF_COOLDOWN_S: z.coerce.number().int().min(0).max(86_400).default(600),

  // --- media ---------------------------------------------------------------
  // Seeing pictures, drawing them, and filming short clips. All three default
  // to the LLM ladder, which serves a `vision` chat ladder, an `image` ladder
  // (OpenAI Images shape, square by default) and a `video` ladder (a job,
  // polled). Seeing is on by default because it is cheap and a bot that
  // cannot see the photo it was sent is a worse bot; drawing and filming are
  // opt-in because each one is a bill. Any part can be pointed at a different
  // OpenAI-compatible endpoint with its own URL and key.
  //
  // Blank disables vision: the account is then told it cannot see the picture
  // rather than being handed a 400 mid-reply.
  VISION_MODEL: z.string().default('vision'),
  VISION_BASE_URL: blankable(z.url()),
  VISION_API_KEY: blankable(z.string().min(1)),
  IMAGE_GEN_ENABLED: offByDefault,
  IMAGE_GEN_MODEL: z.string().min(1).default('image'),
  IMAGE_GEN_BASE_URL: blankable(z.url()),
  IMAGE_GEN_API_KEY: blankable(z.string().min(1)),
  // Only sent to models that take it (gpt-image-*, dall-e-*); the ladder's
  // rungs are many models and the parameter would be a 400 on some of them.
  IMAGE_GEN_QUALITY: blankable(z.string().min(1)),
  VIDEO_GEN_ENABLED: offByDefault,
  // `ladder` speaks POST /video/generations and polls the media.job it gets
  // back; `openai` speaks Sora's POST /videos. The ladder relays Surplus's
  // job object untouched, so the finished shape is read leniently.
  VIDEO_GEN_API: z.enum(['ladder', 'openai']).default('ladder'),
  VIDEO_GEN_MODEL: z.string().min(1).default('video'),
  VIDEO_GEN_BASE_URL: blankable(z.url()),
  VIDEO_GEN_API_KEY: blankable(z.string().min(1)),
  // Clip length, for the APIs that take one. Sora accepts 4, 8 or 12.
  VIDEO_GEN_SECONDS: z.coerce.number().int().min(1).max(60).default(4),
  VIDEO_GEN_TIMEOUT_S: z.coerce.number().int().min(30).max(3_600).default(600),
  // Where a copy of everything generated is written, like charts.
  MEDIA_OUTPUT_DIR: z.string().default('data/media'),
  // Daily ceilings, per kind. The prompt asks the drafter to be sparing; this
  // is what makes "make me a video" from a hundred strangers survivable.
  MEDIA_IMAGES_PER_DAY: z.coerce.number().int().min(0).max(1_000).default(20),
  MEDIA_VIDEOS_PER_DAY: z.coerce.number().int().min(0).max(100).default(3),
  // What Tiny looks like, as the account's published artwork shows it. The
  // vision ladder studies these once, a text step distils them into one
  // character sheet, and the sheet is prepended to every generation prompt
  // that features Tiny. Comma-separated URLs; unset or blank uses the
  // pictures from tinyhumansworld.com; `none` turns the sheet off.
  TINY_REFERENCE_IMAGES: z
    .string()
    .default('')
    .transform((value) => {
      const trimmed = value.trim()
      if (trimmed.toLowerCase() === 'none') return [] as string[]
      const urls = trimmed.split(',').map((entry) => entry.trim()).filter(Boolean)
      return urls.length > 0 ? urls : DEFAULT_REFERENCE_IMAGES
    }),
  REFERENCE_SHEET_PATH: z.string().default('data/media/reference-sheet.json'),

  // --- the wallet ----------------------------------------------------------
  // Nothing reads these today. The agent has no airdrop tool: it was added and
  // removed on 2026-09-08, and `test/capabilities.test.ts` explains why. The
  // settings stay because `src/airdrop/` and `src/clients/wallet.ts` stay, and
  // re-wiring a tool would need them back exactly as they were.
  AIRDROP_ENABLED: offByDefault,
  AIRDROP_DRY_RUN: booleanFromString,
  AIRDROP_PRIVATE_KEY: blankable(
    z.string().regex(/^(0x)?[0-9a-fA-F]{64}$/, 'must be a 32-byte hex private key'),
  ),
  AIRDROP_TOKEN_ADDRESS: blankable(
    z.string().regex(/^0x[0-9a-fA-F]{40}$/, 'must be a contract address'),
  ),
  AIRDROP_TOKEN_SYMBOL: z.string().min(1).default('TINY'),
  AIRDROP_RPC_URL: blankable(z.string()),
  AIRDROPS_COLLECTION: z.string().min(1).default('airdrops'),
  // The ceilings. A turn cannot raise any of them by asking.
  AIRDROP_MAX_PER_AIRDROP: z.coerce.number().int().min(1).default(100),
  // What the account has publicly promised. The model still chooses, but it is
  // told the number it has committed to in front of everybody: the campaign
  // post says 100 and it was sending 10, which is worse than sending nothing.
  AIRDROP_STANDARD_AMOUNT: z.coerce.number().int().min(1).default(100),
  AIRDROP_MAX_PER_DAY: z.coerce.number().int().min(1).default(1000),
  AIRDROP_MAX_PER_RECIPIENT: z.coerce.number().int().min(1).default(1),
  // Off, this pays any address anywhere in the turn -- including one that came
  // from a search result or a recalled memory rather than from the person
  // being answered. Leave it on unless you know why you are turning it off.
  AIRDROP_REQUIRE_ADDRESS_IN_POST: booleanFromString,
  // A weak signal deliberately: a follow costs nothing and a farm can buy a
  // thousand. It filters drive-by begging; the caps are what make a bad day
  // survivable.
  AIRDROP_REQUIRE_FOLLOW: booleanFromString,
  RECIPIENTS_COLLECTION: z.string().min(1).default('recipients'),
})

export type Config = z.infer<typeof schema>

export function loadConfig(environment: NodeJS.ProcessEnv = process.env): Config {
  return schema.parse(environment)
}
