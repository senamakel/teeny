import 'dotenv/config'
import cron from 'node-cron'
import { XApiClient } from './clients/x.js'
import { OpenAICompatibleModel } from './clients/llm.js'
import { loadConfig } from './config.js'
import { ReplyHarness } from './harness/reply-harness.js'
import { IngestJob } from './jobs/ingest-job.js'
import { ReplyJob } from './jobs/reply-job.js'
import { ReplyWorker } from './jobs/reply-worker.js'
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

const config = loadConfig()
const memory = new CortexMemory(
  {
    apiUrl: config.CORTEX_API_URL,
    actor: config.CORTEX_ACTOR,
    bearer: config.CORTEX_TOKEN,
  },
  config.CORTEX_SCOPE,
)
const x = new XApiClient({
  username: config.X_BOT_USERNAME,
  apiKey: config.X_API_KEY,
  apiSecret: config.X_API_SECRET,
  accessToken: config.X_ACCESS_TOKEN,
  accessTokenSecret: config.X_ACCESS_TOKEN_SECRET,
  ...(config.X_BEARER_TOKEN ? { bearerToken: config.X_BEARER_TOKEN } : {}),
})
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

const agentOptions: AgentOptions = {
  memory,
  x,
  persona,
  ...(marketAccess ? { market: marketAccess } : {}),
  maxEmoji: config.MAX_EMOJI_PER_POST,
  botUsername,
  dryRun: config.DRY_RUN,
  llm: {
    baseUrl: config.LLM_BASE_URL,
    apiKey: config.LLM_API_KEY,
    model: config.LLM_MODEL,
  },
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
    : new OpenAICompatibleModel({
        baseUrl: config.LLM_BASE_URL,
        apiKey: config.LLM_API_KEY,
        model: config.LLM_MODEL,
        botUsername,
      })
const queue = new MongoMentionQueue({
  uri: config.MONGO_URI,
  database: config.MONGO_DATABASE,
  collection: config.MENTIONS_COLLECTION,
})
const app = buildServer({
  memory,
  consumerSecret: config.X_API_SECRET,
  logger: { level: config.LOG_LEVEL },
  ...(useAgent ? { agent: new LangGraphAgent(agentOptions) } : {}),
  ...(config.AGENT_API_TOKEN ? { agentToken: config.AGENT_API_TOKEN } : {}),
  queue,
  botUserId: () => x.userId(),
  botUsername,
})
const role = config.SERVICE_ROLE
const runsIngest = role === 'all' || role === 'ingest'
const runsReplies = role === 'all' || role === 'reply'
// The chart agent publishes on its own timer. It is opt-in and single-homed:
// two containers on the same schedule would post the same chart twice.
const runsChart = config.CHART_ENABLED && (role === 'all' || role === 'chart')

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
  })
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
    }),
    symbol: config.CHART_TOKEN_SYMBOL,
    timeframe: config.CHART_TIMEFRAME,
    logger: app.log,
  })
  cron.schedule(config.CHART_CRON, () => {
    void chartJob.run().catch((error: unknown) => app.log.error({ err: error }, 'chart job failed'))
  })
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
    giphy: gifs ? config.GIPHY_CHANNEL : false,
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
    chart: runsChart
      ? `${config.CHART_TOKEN_SYMBOL} ${config.CHART_TIMEFRAME} at "${config.CHART_CRON}" via ${config.CHART_RENDERER}`
      : false,
  },
  'teeny started',
)
