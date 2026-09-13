import 'dotenv/config'
import { loadConfig } from '../config.js'
import { AgentChatModel } from '../agent/chat-model.js'
import { loadPersona } from '../agent/soul.js'
import { GiphyClient } from '../clients/giphy.js'
import { TelegramApiClient, type TelegramApi, type TelegramUpdate } from '../clients/telegram.js'
import { XApiClient } from '../clients/x.js'
import { ChatHarness } from '../harness/chat-harness.js'
import { GlobalMarketClient } from '../market/global.js'
import { TokenMarket } from '../market/market.js'
import { CortexMemory } from '../memory/cortex.js'
import { TelegramBot } from '../telegram/bot.js'

/**
 * Rehearses a Telegram turn without a Telegram user: the message comes from
 * the command line, the drafter is the real one, and what it would have sent
 * is printed instead of sent.
 *
 *   npm run telegram-dry-run -- "how is $TINY doing today"
 *   npm run telegram-dry-run -- --group "@teenythebot gm"
 */
const config = loadConfig()
if (!config.TELEGRAM_BOT_TOKEN) throw new Error('TELEGRAM_BOT_TOKEN is not set')

const args = process.argv.slice(2)
const group = args.includes('--group')
const text = args.filter((arg) => arg !== '--group').join(' ') || 'hi, who are you?'

const telegram = new TelegramApiClient({ token: config.TELEGRAM_BOT_TOKEN })
const me = await telegram.getMe()
if (!me.username) throw new Error('the bot has no username')
console.log(`bot: @${me.username} (${me.id})`)

const memory = new CortexMemory(
  { apiUrl: config.CORTEX_API_URL, actor: config.CORTEX_ACTOR, bearer: config.CORTEX_TOKEN },
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
const gifs = config.GIPHY_API_KEY
  ? {
      source: new GiphyClient({ apiKey: config.GIPHY_API_KEY, channel: config.GIPHY_CHANNEL }),
      channel: config.GIPHY_CHANNEL,
    }
  : undefined
const market = config.CHART_TOKEN_ADDRESS
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

const model = new AgentChatModel({
  memory,
  x,
  persona: loadPersona({
    soulPath: config.SOUL_PATH,
    contextPath: config.CONTEXT_PATH,
    skillsDir: config.SKILLS_DIR,
  }),
  ...(market ? { market } : {}),
  maxEmoji: config.MAX_EMOJI_PER_POST,
  botUsername: config.X_BOT_USERNAME,
  channelHandle: me.username,
  dryRun: true,
  llm: { baseUrl: config.LLM_BASE_URL, apiKey: config.LLM_API_KEY, model: config.LLM_MODEL },
  maxSteps: config.AGENT_MAX_STEPS,
  ...(config.EXA_API_KEY ? { exaApiKey: config.EXA_API_KEY } : {}),
  ...(gifs ? { gifs } : {}),
  searchLimit: config.X_SEARCH_MAX_RESULTS,
  ownTokenAddresses: config.OWN_TOKEN_ADDRESSES,
})

// Nothing is sent: the API the loop sees records instead.
const recording: TelegramApi = {
  getMe: () => telegram.getMe(),
  getUpdates: async () => [],
  sendMessage: async (chatId, body, options) => {
    console.log(`\n--- would send to ${chatId} ${JSON.stringify(options ?? {})}\n${body}\n---`)
    return { id: 1 }
  },
  sendAnimation: async (_chatId, url) => {
    console.log(`--- would send GIF ${url}`)
    return { id: 2 }
  },
  sendChatAction: async () => undefined,
}

const logger = {
  info: (...a: unknown[]) => console.log('[info]', ...a),
  warn: (...a: unknown[]) => console.warn('[warn]', ...a),
  error: (...a: unknown[]) => console.error('[error]', ...a),
  debug: () => undefined,
} as unknown as ConstructorParameters<typeof TelegramBot>[0]['logger']

const bot = new TelegramBot({
  api: recording,
  harness: new ChatHarness(memory, model),
  logger,
  bot: { id: me.id, username: me.username },
  // Not the bot's own dry run: that would skip the send path this rehearses.
  dryRun: false,
  ...(gifs ? { gifs: gifs.source } : {}),
  maxEmoji: config.MAX_EMOJI_PER_POST,
})

const update: TelegramUpdate = {
  update_id: 1,
  message: {
    message_id: Date.now() % 1_000_000,
    date: Math.floor(Date.now() / 1_000),
    chat: group
      ? { id: -1, type: 'supergroup', title: 'rehearsal' }
      : { id: 1, type: 'private' },
    from: { id: 1, is_bot: false, first_name: 'Rehearsal', username: 'rehearsal' },
    text,
    ...(group
      ? { entities: [{ type: 'mention', offset: text.indexOf(`@${me.username}`), length: me.username.length + 1 }] }
      : {}),
  },
}
console.log(`\n>>> ${group ? 'group' : 'DM'}: ${text}`)
await bot.handle(update)
process.exit(0)
