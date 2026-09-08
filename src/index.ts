import 'dotenv/config'
import cron from 'node-cron'
import { XApiClient } from './clients/x.js'
import { OpenAICompatibleModel } from './clients/llm.js'
import { loadConfig } from './config.js'
import { ReplyHarness } from './harness/reply-harness.js'
import { IngestJob } from './jobs/ingest-job.js'
import { ReplyJob } from './jobs/reply-job.js'
import { MongoMentionQueue } from './queue/mentions.js'
import { MongoAirdropLedger } from './airdrop/ledger.js'
import { MongoRecipientRegistry } from './airdrop/registry.js'
import { Erc20Wallet } from './clients/wallet.js'
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
 * The wallet, if one is configured.
 *
 * Three things have to be true before a signer exists at all: the switch is on,
 * there is a key, and there is a token to send. Any of them missing and the
 * agent is built without the tool rather than with a tool that fails at the
 * moment somebody is owed an answer.
 */
const airdrop = (() => {
  if (!config.AIRDROP_ENABLED) return undefined
  if (!config.AIRDROP_PRIVATE_KEY || !config.AIRDROP_TOKEN_ADDRESS) {
    console.error(
      'AIRDROP_ENABLED is true but AIRDROP_PRIVATE_KEY or AIRDROP_TOKEN_ADDRESS is missing; ' +
        'starting with no wallet.',
    )
    return undefined
  }
  const wallet = new Erc20Wallet({
    privateKey: config.AIRDROP_PRIVATE_KEY,
    token: config.AIRDROP_TOKEN_ADDRESS,
    ...(config.AIRDROP_RPC_URL ? { rpcUrl: config.AIRDROP_RPC_URL } : {}),
  })
  return {
    wallet,
    ledger: new MongoAirdropLedger({
      uri: config.MONGO_URI,
      database: config.MONGO_DATABASE,
      collection: config.AIRDROPS_COLLECTION,
    }),
    registry: new MongoRecipientRegistry({
      uri: config.MONGO_URI,
      database: config.MONGO_DATABASE,
      collection: config.RECIPIENTS_COLLECTION,
    }),
    followsAccount: (userId: string) => x.followsAccount(userId),
    limits: {
      maxPerAirdrop: config.AIRDROP_MAX_PER_AIRDROP,
      maxPerDay: config.AIRDROP_MAX_PER_DAY,
      maxPerRecipient: config.AIRDROP_MAX_PER_RECIPIENT,
      requireAddressInPost: config.AIRDROP_REQUIRE_ADDRESS_IN_POST,
      requireFollow: config.AIRDROP_REQUIRE_FOLLOW,
    },
    symbol: config.AIRDROP_TOKEN_SYMBOL,
    dryRun: config.AIRDROP_DRY_RUN,
  }
})()

const agentOptions: AgentOptions = {
  memory,
  x,
  persona,
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
  ...(airdrop ? { airdrop } : {}),
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
    botUserId: () => x.userId(),
    ...(gifs ? { gifs: gifs.source } : {}),
  })
  cron.schedule(config.REPLY_CRON, () => {
    void replyJob.run().catch((error: unknown) => app.log.error({ err: error }, 'reply job failed'))
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
    soul: Boolean(persona.soul),
    context: Boolean(persona.context),
    skills: persona.skills.map((skill) => skill.name),
    exa: Boolean(config.EXA_API_KEY),
    giphy: gifs ? config.GIPHY_CHANNEL : false,
    role,
    ingestCron: runsIngest ? config.INGEST_CRON : false,
    replyCron: runsReplies ? config.REPLY_CRON : false,
    agentEndpoint: Boolean(useAgent && config.AGENT_API_TOKEN),
    wallet: airdrop
      ? {
          address: airdrop.wallet.address,
          token: config.AIRDROP_TOKEN_SYMBOL,
          dryRun: airdrop.dryRun,
          perAirdrop: airdrop.limits.maxPerAirdrop,
          perDay: airdrop.limits.maxPerDay,
          perRecipient: airdrop.limits.maxPerRecipient,
          addressMustBeInPost: airdrop.limits.requireAddressInPost,
          mustFollow: airdrop.limits.requireFollow,
        }
      : false,
  },
  'teeny started',
)
