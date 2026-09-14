import 'dotenv/config'
import cron from 'node-cron'
import { XApiClient } from './clients/x.js'
import { OpenAICompatibleModel } from './clients/llm.js'
import { loadConfig } from './config.js'
import { ReplyHarness } from './harness/reply-harness.js'
import { IngestJob } from './jobs/ingest-job.js'
import { ReplyJob } from './jobs/reply-job.js'
import { ReplyWorker } from './jobs/reply-worker.js'
import { MongoJournal } from './journal/journal.js'
import { MongoMentionQueue } from './queue/mentions.js'
import { buildChartAgent } from './agent/chart-wiring.js'
import { GlobalMarketClient } from './market/global.js'
import { TokenMarket } from './market/market.js'
import { ChartJob } from './jobs/chart-job.js'
import { LangGraphAgent, type AgentOptions } from './agent/graph.js'
import { loadPersona } from './agent/soul.js'
import { AgentReplyModel } from './agent/reply-model.js'
import { GiphyClient } from './clients/giphy.js'
import { CortexMemory } from './memory/cortex.js'
import { buildServer } from './server.js'
import { FileStateStore } from './state/file-state.js'
import { AgentChatModel } from './agent/chat-model.js'
import { TelegramApiClient } from './clients/telegram.js'
import { ChatHarness } from './harness/chat-harness.js'
import { TelegramBot } from './telegram/bot.js'
import { MongoChatTranscript } from './telegram/transcript.js'
import { buildMedia } from './media/wiring.js'
import { buildMorningPipeline } from './morning/wiring.js'
import { MorningJob, MORNING_CALENDAR_SLOT } from './jobs/morning-job.js'
import { buildMoonPipeline } from './moon/wiring.js'
import { MoonJob } from './jobs/moon-job.js'
import { SafetyGate } from './safety/gate.js'
import { MongoHoldLog, type Hold } from './safety/holds.js'
import { ModelJudge } from './safety/judge.js'
import { AlgoCritic } from './critic/algo.js'
import { ContentAgent } from './agent/content-model.js'
import { loadCalendar } from './calendar/calendar.js'
import { CalendarJob } from './jobs/calendar-job.js'
import { buildWebhookForwarder } from './webhooks/forward.js'
import { Cadence, CadenceHeld, paced } from './cadence/cadence.js'
import { MongoPostLedger } from './cadence/ledger.js'
import { MongoPeople } from './cadence/people.js'
import { ReplyBudget } from './cadence/budget.js'
import { HeatJob } from './cadence/heat.js'
import { MongoLedger } from './ledger/ledger.js'
import { BrainReader } from './ledger/brain.js'

const config = loadConfig()
const memory = new CortexMemory(
  {
    apiUrl: config.CORTEX_API_URL,
    actor: config.CORTEX_ACTOR,
    bearer: config.CORTEX_TOKEN,
  },
  config.CORTEX_SCOPE,
)
const rawX = new XApiClient({
  username: config.X_BOT_USERNAME,
  apiKey: config.X_API_KEY,
  apiSecret: config.X_API_SECRET,
  accessToken: config.X_ACCESS_TOKEN,
  accessTokenSecret: config.X_ACCESS_TOKEN_SECRET,
  ...(config.X_BEARER_TOKEN ? { bearerToken: config.X_BEARER_TOKEN } : {}),
})
// The content calendar's spacing rule, in front of every own post from any
// job or tool: the ledger is what they all write, and the cadence reads it.
const ledger = new MongoPostLedger({ uri: config.MONGO_URI, database: config.MONGO_DATABASE })
const people = new MongoPeople({ uri: config.MONGO_URI, database: config.MONGO_DATABASE })
const gapMs = config.POST_MIN_GAP_HOURS * 3_600_000
const cadence = new Cadence({
  ledger,
  minGapMs: gapMs,
  toleranceMs: config.POST_GAP_TOLERANCE_MINUTES * 60_000,
  // The logger does not exist yet; the closure runs only after `app` does.
  logger: {
    info: (detail, event) => app.log.info(detail, event),
    warn: (detail, event) => app.log.warn(detail, event),
  },
})
const x = paced(rawX, cadence)
/**
 * The account's own handle, read from the token rather than trusted from the
 * environment. `X_BOT_USERNAME` is a fallback for a cold start where X cannot
 * be reached; when the two disagree, the token is right and the environment is
 * stale, which is exactly what happened when the account renamed itself.
 */
const identity = await x.identity().catch((error: unknown) => {
  console.error('could not identify the account from its token:', error)
  return undefined
})
const botUsername = identity?.username ?? config.X_BOT_USERNAME

const gifs = config.GIPHY_API_KEY
  ? {
      source: new GiphyClient({
        apiKey: config.GIPHY_API_KEY,
        channel: config.GIPHY_CHANNEL,
      }),
      channel: config.GIPHY_CHANNEL,
    }
  : undefined
const persona = loadPersona({
  soulPath: config.SOUL_PATH,
  contextPath: config.CONTEXT_PATH,
  skillsDir: config.SKILLS_DIR,
})
/**
 * Read-only market access for the reply drafter and the task agent.
 *
 * Built whenever a token is configured, independently of CHART_ENABLED: the
 * chart job publishes and is opt-in, but being able to answer "how is $TINY
 * doing" is not a publishing capability and the account looked foolish without
 * it.
 */
const marketAccess = config.CHART_TOKEN_ADDRESS
  ? {
      market: new TokenMarket({
        network: config.CHART_CHAIN,
        ...(config.SOLANA_RPC_URL ? { solanaRpcUrl: config.SOLANA_RPC_URL } : {}),
        ...(config.GMGN_BASE_URL ? { gmgnBaseUrl: config.GMGN_BASE_URL } : {}),
        watchlist: config.KOL_WALLETS,
      }),
      address: config.CHART_TOKEN_ADDRESS,
      symbol: config.CHART_TOKEN_SYMBOL,
      global: new GlobalMarketClient(),
    }
  : undefined

/**
 * The inference ledger: one row per model call, every surface, and the
 * running token count the page's counter reads. Every model the process
 * builds is told about it through its llm options, and the raw-fetch
 * clients (the simple reply model, the morning text model, the vision
 * reader) are handed it directly. Same Mongo as the journal.
 */
const inferences = new MongoLedger({ uri: config.MONGO_URI, database: config.MONGO_DATABASE })
const llm = {
  baseUrl: config.LLM_BASE_URL,
  apiKey: config.LLM_API_KEY,
  model: config.LLM_MODEL,
  ledger: inferences,
}

/**
 * One media access for the whole process: the reply drafter, the chat
 * drafter and the task agent draw on the same daily budget and leave their
 * files in the same store, which is what lets a job attach what a turn made.
 */
// The logger does not exist yet; the closure runs only after `app` does.
const media = buildMedia(config, {
  log: (event, detail) => app.log.info(detail ?? {}, event),
  ledger: inferences,
})

/**
 * One content filter for the whole process. Every surface gets the same one:
 * the reply job and the Telegram bot gate their own drafts, the task and
 * chart agents' publish tools gate theirs, and the media tools gate their
 * prompts. The judge rides on the chat endpoint unless pointed elsewhere.
 */
// Everything the filter holds, kept for review. Same Mongo as the queue and
// the journal, so every role can write to it and the API role can serve it.
const holds = new MongoHoldLog({
  uri: config.MONGO_URI,
  database: config.MONGO_DATABASE,
  collection: config.HOLDS_COLLECTION,
})
const safety = new SafetyGate({
  holds,
  ...(config.SAFETY_JUDGE
    ? {
        judge: new ModelJudge({
          baseUrl: config.LLM_BASE_URL,
          apiKey: config.LLM_API_KEY,
          model: config.SAFETY_JUDGE_MODEL ?? config.LLM_MODEL,
          ledger: inferences,
        }),
      }
    : {}),
  extraAllowedLinks: config.LINK_ALLOWLIST,
  // The logger does not exist yet; the closure runs only after `app` does.
  logger: {
    info: (detail, event) => app.log.info(detail, event),
    warn: (detail, event) => app.log.warn(detail, event),
  },
})

/**
 * The ranker's reader, one for the process, and only for the pipelines that
 * generate a timeline post: the task and content agents get it as a tool, the
 * chart agent too, the morning pipeline reads its script and caption with it,
 * and the API serves it so anything else can ask. The reply drafter and the
 * Telegram bot never see it: a reply is already a conversation, and a reader
 * that scores it for the timeline would pull it toward being a post.
 */
const critic = config.ALGO_CRITIC
  ? new AlgoCritic({
      baseUrl: config.LLM_BASE_URL,
      apiKey: config.LLM_API_KEY,
      model: config.ALGO_CRITIC_MODEL,
      ledger: inferences,
    })
  : undefined

const agentOptions: AgentOptions = {
  memory,
  x,
  persona,
  safety,
  ...(critic ? { critic } : {}),
  ...(marketAccess ? { market: marketAccess } : {}),
  ...(media ? { media } : {}),
  maxEmoji: config.MAX_EMOJI_PER_POST,
  botUsername,
  dryRun: config.DRY_RUN,
  llm,
  maxSteps: config.AGENT_MAX_STEPS,
  ...(config.EXA_API_KEY ? { exaApiKey: config.EXA_API_KEY } : {}),
  ...(gifs ? { gifs } : {}),
  searchLimit: config.X_SEARCH_MAX_RESULTS,
  ownTokenAddresses: config.OWN_TOKEN_ADDRESSES,
}
const useAgent = config.AGENT_MODE === 'langgraph'
const model =
  useAgent
    ? new AgentReplyModel(agentOptions)
    : new OpenAICompatibleModel({ ...llm, botUsername })
const queue = new MongoMentionQueue({
  uri: config.MONGO_URI,
  database: config.MONGO_DATABASE,
  collection: config.MENTIONS_COLLECTION,
})

// Telegram is built later, if this role runs it; the release hook needs it
// to send an approved chat draft, so it is a slot filled in below.
let telegramApi: TelegramApiClient | undefined

/**
 * What approving a withheld draft does: publishes it as written, on the
 * surface it was written for. Honours DRY_RUN like every other publish.
 */
async function release(hold: Hold): Promise<string> {
  const where = hold.context
  if (where.surface === 'x' || where.surface === 'chart') {
    if (config.DRY_RUN) return 'dry run: would have published on X'
    try {
      const posted = await x.createPost(hold.text, where.postId ? { replyToPostId: where.postId } : {})
      return `published on X as ${posted.id}`
    } catch (error) {
      // An own post released into a slot the cadence refuses is recorded,
      // not published; the operator can release it again after the gap.
      if (error instanceof CadenceHeld) return `recorded, not published: ${error.message}`
      throw error
    }
  }
  if (where.surface === 'telegram' && where.chatId) {
    if (!telegramApi) return 'recorded; this process has no Telegram client to send it with'
    if (config.TELEGRAM_DRY_RUN) return 'dry run: would have sent on Telegram'
    const sent = await telegramApi.sendMessage(Number(where.chatId), hold.text, {
      ...(where.messageId ? { replyToMessageId: Number(where.messageId) } : {}),
      ...(where.messageThreadId ? { messageThreadId: Number(where.messageThreadId) } : {}),
    })
    return `sent on Telegram as ${sent.id}`
  }
  // A calendar post is not released after the fact: the text alone is not
  // the post (the picture it was held with is gone), and the item runs
  // again on its next cron anyway.
  if (where.surface === 'calendar') {
    return `recorded; the "${where.tool ?? 'calendar'}" item posts again on its next run`
  }
  // Nor is the morning post: the caption was written for a picture and a
  // clip that are not kept past the run, and tomorrow's runs on its own.
  if (where.surface === 'morning') {
    return 'recorded; the morning post runs again tomorrow'
  }
  // Nor the moon chart: the picture was drawn from that minute's prices.
  if (where.surface === 'moon') {
    return 'recorded; the moon chart runs again tomorrow'
  }
  return 'recorded; nothing here can publish it'
}
// Every role writes to it and the ingest role serves it, so it lives in the
// Mongo all three containers already share.
const journal = new MongoJournal({ uri: config.MONGO_URI, database: config.MONGO_DATABASE })
const forwarder = config.WEBHOOK_FORWARD_URL
  ? buildWebhookForwarder({
      url: config.WEBHOOK_FORWARD_URL,
      ...(config.WEBHOOK_FORWARD_TOKEN ? { token: config.WEBHOOK_FORWARD_TOKEN } : {}),
      // The logger does not exist yet; the closure runs only after `app` does.
      logger: {
        info: (detail, event) => app.log.info(detail, event),
        warn: (detail, event) => app.log.warn(detail, event),
      },
    })
  : undefined
/**
 * The brain's checkout, if there is one at the path: the agents' spend is
 * folded into the ledger on a timer, and the roster and runs are served.
 */
const brain = config.BRAIN_DATA_DIR
  ? new BrainReader({ dir: config.BRAIN_DATA_DIR, slug: config.BRAIN_COMPANY })
  : undefined
const app = buildServer({
  memory,
  consumerSecret: config.X_API_SECRET,
  ledger: inferences,
  ...(brain ? { brain } : {}),
  ...(forwarder ? { forwarder } : {}),
  people,
  logger: { level: config.LOG_LEVEL },
  ...(useAgent ? { agent: new LangGraphAgent({ ...agentOptions, ...(critic ? { critic } : {}) }) } : {}),
  ...(config.AGENT_API_TOKEN ? { agentToken: config.AGENT_API_TOKEN } : {}),
  queue,
  botUserId: () => x.userId(),
  botUsername,
  mind: {
    journal,
    status: { mode: config.AGENT_MODE, model: config.LLM_MODEL, dryRun: config.DRY_RUN },
  },
  safety,
  review: { holds, release },
  ...(critic ? { critic } : {}),
})
// The character sheet takes a dozen model calls to build the first time.
// Started now so the first "draw me a Tiny" of the day does not pay for it.
media?.reference?.warm()

const role = config.SERVICE_ROLE
const runsIngest = role === 'all' || role === 'ingest'
// The brain's spend is folded by whichever process serves the page's feed,
// which is the ingest role: one folder, or the same sample is offered twice
// (harmless, the ledger keeps one) by two processes for nothing.
if (brain && runsIngest) {
  const fold = async (): Promise<void> => {
    if (!(await brain.exists())) return
    const offered = await brain.foldUsage(inferences)
    if (offered) app.log.debug({ offered }, 'brain usage folded')
  }
  void fold().catch((error: unknown) => app.log.warn({ err: error }, 'brain usage fold failed'))
  cron.schedule(config.BRAIN_USAGE_CRON, () => {
    void fold().catch((error: unknown) => app.log.warn({ err: error }, 'brain usage fold failed'))
  })
}
const runsReplies = role === 'all' || role === 'reply'
// The chart agent publishes on its own timer. It is opt-in and single-homed:
// two containers on the same schedule would post the same chart twice.
const runsChart = config.CHART_ENABLED && (role === 'all' || role === 'chart')
// The morning post too: one timer, one process, or it says good morning twice.
const rota = { days: config.POST_ROTA_DAYS, anchor: config.POST_ROTA_ANCHOR }
const runsMorning = config.MORNING_ENABLED && (role === 'all' || role === 'morning')
// And the moon chart, for the same reason.
const runsMoon = config.MOON_ENABLED && (role === 'all' || role === 'moon')
// The content calendar publishes on its own timers and, with the morning
// post, is where the account generates media unasked. Opt-in and
// single-homed, like the chart.
const runsCalendar = config.CALENDAR_ENABLED && (role === 'all' || role === 'calendar')
// Telegram lives in the ingest process. It long-polls, and Telegram allows one
// poller per token, so it rides with the role that already runs in exactly one
// place rather than getting a container of its own.
const runsTelegram = Boolean(config.TELEGRAM_BOT_TOKEN) && runsIngest

if (runsIngest) {
  const ingestJob = new IngestJob({
    x,
    queue,
    state: new FileStateStore(new URL('../data/state.json', import.meta.url).pathname),
    logger: app.log,
    maxResults: config.X_SEARCH_MAX_RESULTS,
    lookbackMinutes: config.MENTION_LOOKBACK_MINUTES,
    botUserId: () => x.userId(),
    botUsername,
  })
  cron.schedule(config.INGEST_CRON, () => {
    void ingestJob.run().catch((error: unknown) => app.log.error({ err: error }, 'ingest failed'))
  })
}

if (runsReplies) {
  const replyJob = new ReplyJob({
    x,
    harness: new ReplyHarness(
      memory,
      model,
      config.THREAD_CONTEXT_MAX_POSTS > 0
        ? { x, maxPosts: config.THREAD_CONTEXT_MAX_POSTS }
        : undefined,
      media?.reader,
      safety,
    ),
    queue,
    logger: app.log,
    dryRun: config.DRY_RUN,
    maxReplies: config.MAX_REPLIES_PER_RUN,
    maxEmoji: config.MAX_EMOJI_PER_POST,
    minReplyIntervalMs: config.REPLY_MIN_INTERVAL_MS,
    maxReplyIntervalMs: config.REPLY_MAX_INTERVAL_MS,
    botUserId: () => x.userId(),
    ...(gifs ? { gifs: gifs.source } : {}),
    ...(media ? { media: media.store } : {}),
    journal,
    safety,
    budget: new ReplyBudget({
      ledger,
      queue,
      autoPerPost: config.REPLY_AUTO_PER_POST,
      smartPerPost: config.REPLY_SMART_PER_POST,
      gapMs,
      botUsername,
    }),
    people,
    likeMentions: config.REPLY_LIKE_MENTIONS,
  })
  if (config.HEAT_ENABLED) {
    const heat = new HeatJob({
      ledger,
      x,
      lookbackMs: 24 * 3_600_000,
      gapMs,
      impressionsPerHour: config.HOT_IMPRESSIONS_PER_HOUR,
      baselineMultiple: config.HOT_BASELINE_MULTIPLE,
      maxHoldSlots: config.HOT_MAX_HOLD_SLOTS,
      logger: app.log,
    })
    cron.schedule(config.HEAT_CRON, () => {
      void heat.run().catch((error: unknown) => app.log.error({ err: error }, 'heat job failed'))
    })
  }
  if (config.REPLY_MODE === 'worker') {
    new ReplyWorker({
      job: replyJob,
      logger: app.log,
      idleMs: config.REPLY_IDLE_MS,
    }).start()
  } else {
    cron.schedule(config.REPLY_CRON, () => {
      void replyJob
        .run()
        .catch((error: unknown) => app.log.error({ err: error }, 'reply job failed'))
    })
  }
}

if (runsChart) {
  const chartJob = new ChartJob({
    agent: buildChartAgent(config, {
      memory,
      x,
      botUsername,
      persona,
      dryRun: config.DRY_RUN,
      safety,
      ...(critic ? { critic } : {}),
      ledger: inferences,
    }),
    symbol: config.CHART_TOKEN_SYMBOL,
    timeframe: config.CHART_TIMEFRAME,
    logger: app.log,
    dryRun: config.DRY_RUN,
    journal,
    rota: { rota, slot: config.CHART_ROTA_SLOT },
    cadence,
  })
  cron.schedule(config.CHART_CRON, () => {
    void chartJob.run().catch((error: unknown) => app.log.error({ err: error }, 'chart job failed'))
  })
}

if (runsMorning) {
  const morningJob = new MorningJob({
    pipeline: buildMorningPipeline(config, {
      memory,
      x,
      media,
      persona,
      botUsername,
      dryRun: config.DRY_RUN,
      logger: app.log,
      safety,
      ...(critic ? { critic } : {}),
      ledger: inferences,
    }),
    logger: app.log,
    journal,
    cadence,
  })
  cron.schedule(config.MORNING_CRON, () => {
    void morningJob
      .run()
      .catch((error: unknown) => app.log.error({ err: error }, 'morning job failed'))
  })
}

if (runsMoon) {
  const moonJob = new MoonJob({
    pipeline: buildMoonPipeline(config, {
      memory,
      x,
      persona,
      botUsername,
      dryRun: config.DRY_RUN,
      logger: app.log,
      safety,
      ledger: inferences,
    }),
    logger: app.log,
    journal,
    rota: { rota, slot: config.MOON_ROTA_SLOT },
    cadence,
  })
  cron.schedule(config.MOON_CRON, () => {
    void moonJob.run().catch((error: unknown) => app.log.error({ err: error }, 'moon job failed'))
  })
}

const calendarItems: string[] = []
if (runsCalendar) {
  const calendar = loadCalendar(config.CALENDAR_PATH)
  // A sending-only client, separate from the bot's poller: sending does not
  // hold the token the way long polling does, so this may run in a process
  // that is not the Telegram one.
  const telegram = config.TELEGRAM_BOT_TOKEN
    ? new TelegramApiClient({ token: config.TELEGRAM_BOT_TOKEN })
    : undefined
  const calendarJob = new CalendarJob({
    agent: new ContentAgent({ ...agentOptions, ...(critic ? { critic } : {}) }),
    memory,
    x,
    ...(telegram ? { telegram } : {}),
    ...(media ? { media: media.store } : {}),
    logger: app.log,
    dryRun: config.DRY_RUN,
    telegramChatIds: config.CALENDAR_TELEGRAM_CHAT_IDS,
    maxEmoji: config.MAX_EMOJI_PER_POST,
    journal,
    safety,
    cadence,
  })
  for (const item of calendar.items) {
    if (!item.enabled) continue
    // The morning job owns the good-morning slot; see MORNING_CALENDAR_SLOT.
    if (runsMorning && item.name === MORNING_CALENDAR_SLOT) {
      app.log.warn(
        { item: item.name, cron: item.cron },
        'calendar: skipping the item, the morning job posts the good morning',
      )
      continue
    }
    calendarItems.push(`${item.name} "${item.cron}"`)
    cron.schedule(
      item.cron,
      () => {
        void calendarJob
          .run(item)
          .catch((error: unknown) => app.log.error({ err: error, item: item.name }, 'calendar job failed'))
      },
      calendar.timezone ? { timezone: calendar.timezone } : {},
    )
  }
}

let telegramHandle: string | false = false
if (runsTelegram && config.TELEGRAM_BOT_TOKEN) {
  const telegram = new TelegramApiClient({ token: config.TELEGRAM_BOT_TOKEN })
  telegramApi = telegram
  // The bot's own handle comes from the token, as the X handle does. Without
  // it the bot cannot tell which group messages are for it, so a failure here
  // is fatal rather than something to poll past.
  const me = await telegram.getMe()
  if (!me.username) throw new Error('the Telegram bot has no username; set one with @BotFather')
  telegramHandle = me.username
  const bot = new TelegramBot({
    api: telegram,
    harness: new ChatHarness(
      memory,
      new AgentChatModel({ ...agentOptions, channelHandle: me.username }),
    ),
    logger: app.log,
    bot: { id: me.id, username: me.username },
    dryRun: config.TELEGRAM_DRY_RUN,
    ...(gifs ? { gifs: gifs.source } : {}),
    ...(media?.reader ? { vision: media.reader } : {}),
    ...(media ? { media: media.store } : {}),
    maxEmoji: config.MAX_EMOJI_PER_POST,
    maxMessageAgeMs: config.TELEGRAM_MAX_MESSAGE_AGE_S * 1_000,
    pollTimeoutSeconds: config.TELEGRAM_POLL_TIMEOUT_S,
    transcript: new MongoChatTranscript({
      uri: config.MONGO_URI,
      database: config.MONGO_DATABASE,
      collection: config.TRANSCRIPT_COLLECTION,
    }),
    historyTokens: config.TELEGRAM_HISTORY_TOKENS,
    debounceMs: config.TELEGRAM_DEBOUNCE_MS,
    gifCooldownMs: config.TELEGRAM_GIF_COOLDOWN_S * 1_000,
    safety,
  })
  void bot.start()
  const shutdown = (): void => {
    bot.stop()
    void bot.drain().finally(() => process.exit(0))
  }
  process.once('SIGTERM', shutdown)
  process.once('SIGINT', shutdown)
}

await app.listen({ host: '0.0.0.0', port: config.PORT })
app.log.info(
  {
    handle: botUsername,
    threadContext: config.THREAD_CONTEXT_MAX_POSTS || false,
    userId: identity?.id ?? 'UNRESOLVED',
    configuredHandle:
      identity && identity.username !== config.X_BOT_USERNAME ? config.X_BOT_USERNAME : undefined,
    dryRun: config.DRY_RUN,
    cron: config.REPLY_CRON,
    agentMode: config.AGENT_MODE,
    market: marketAccess ? `${config.CHART_TOKEN_SYMBOL} read-only` : false,
    soul: Boolean(persona.soul),
    context: Boolean(persona.context),
    skills: persona.skills.map((skill) => skill.name),
    exa: Boolean(config.EXA_API_KEY),
    safety: config.SAFETY_JUDGE
      ? `lexicon + links + judge on ${config.SAFETY_JUDGE_MODEL ?? config.LLM_MODEL}`
      : 'lexicon + links (no judge)',
    allowedLinks: config.LINK_ALLOWLIST.length,
    review: config.AGENT_API_TOKEN ? '/safety' : 'no AGENT_API_TOKEN, review page off',
    critic: critic ? `x_critique + /critique on ${config.ALGO_CRITIC_MODEL}` : 'off',
    giphy: gifs ? config.GIPHY_CHANNEL : false,
    vision: media?.reader ? config.VISION_MODEL : false,
    referenceSheet: media?.reference ? config.TINY_REFERENCE_IMAGES.length : false,
    imageGen: media?.images ? `${config.IMAGE_GEN_MODEL}, ${config.MEDIA_IMAGES_PER_DAY}/day` : false,
    videoGen: media?.videos ? `${config.VIDEO_GEN_MODEL}, ${config.MEDIA_VIDEOS_PER_DAY}/day` : false,
    role,
    ingestCron: runsIngest ? config.INGEST_CRON : false,
    replyGapMs: runsReplies
      ? `${config.REPLY_MIN_INTERVAL_MS}-${config.REPLY_MAX_INTERVAL_MS} random`
      : false,
    replies: runsReplies
      ? config.REPLY_MODE === 'worker'
        ? `worker, ${config.REPLY_IDLE_MS}ms idle`
        : config.REPLY_CRON
      : false,
    agentEndpoint: Boolean(useAgent && config.AGENT_API_TOKEN),
    telegram: telegramHandle
      ? `@${telegramHandle}${config.TELEGRAM_DRY_RUN ? ' (dry run)' : ''}`
      : false,
    chart: runsChart
      ? `${config.CHART_TOKEN_SYMBOL} ${config.CHART_TIMEFRAME} at "${config.CHART_CRON}" via ${config.CHART_RENDERER}, rota slot ${config.CHART_ROTA_SLOT}`
      : false,
    morning: runsMorning
      ? `at "${config.MORNING_CRON}", ${media?.videos ? 'video' : 'image'}, ${config.MORNING_NEWS_TOPICS.length} topics`
      : false,
    moon: runsMoon ? `${config.MOON_COIN_ID} against ${config.MOON_CATEGORY} at "${config.MOON_CRON}", rota slot ${config.MOON_ROTA_SLOT}` : false,
    rota: runsChart || runsMoon ? `${config.POST_ROTA_DAYS} days from ${config.POST_ROTA_ANCHOR}` : false,
    cadence: `${config.POST_MIN_GAP_HOURS}h between own posts, ${config.POST_GAP_TOLERANCE_MINUTES}m tolerance`,
    heat: runsReplies && config.HEAT_ENABLED
      ? `"${config.HEAT_CRON}", hot at ${config.HOT_IMPRESSIONS_PER_HOUR}/h and ${config.HOT_BASELINE_MULTIPLE}x, hold up to ${config.HOT_MAX_HOLD_SLOTS} slots`
      : false,
    replyBudget: runsReplies
      ? `${config.REPLY_AUTO_PER_POST} auto + ${config.REPLY_SMART_PER_POST} smart per post${config.REPLY_LIKE_MENTIONS ? ', likes unbudgeted' : ''}`
      : false,
    calendar: runsCalendar ? calendarItems : false,
  },
  'teeny started',
)
