import 'dotenv/config'
import { ContentAgent } from '../agent/content-model.js'
import { loadPersona } from '../agent/soul.js'
import { loadCalendar } from '../calendar/calendar.js'
import { TelegramApiClient } from '../clients/telegram.js'
import { XApiClient } from '../clients/x.js'
import { loadConfig } from '../config.js'
import { CalendarJob } from '../jobs/calendar-job.js'
import { GlobalMarketClient } from '../market/global.js'
import { TokenMarket } from '../market/market.js'
import { buildMedia } from '../media/wiring.js'
import { CortexMemory } from '../memory/cortex.js'
import { lexicalGate } from '../safety/gate.js'

/**
 * Runs one calendar item now, dry: the real drafter, the real generators if
 * they are enabled (and the day's budget is spent accordingly), and nothing
 * published. What would have gone out is printed, and the picture is in
 * MEDIA_OUTPUT_DIR.
 *
 *   npm run calendar-dry-run -- gm
 *   npm run calendar-dry-run -- gm --publish   # actually posts, if DRY_RUN=false
 */
const config = loadConfig()
const args = process.argv.slice(2)
const publish = args.includes('--publish')
const name = args.find((arg) => !arg.startsWith('--'))
const calendar = loadCalendar(config.CALENDAR_PATH)
const item = calendar.items.find((entry) => entry.name === name)
if (!item) {
  console.error(`pick an item: ${calendar.items.map((entry) => entry.name).join(', ')}`)
  process.exit(1)
}

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
const media = buildMedia(config, { log: (event, detail) => console.log('[media]', event, detail ?? '') })
const dryRun = !publish || config.DRY_RUN
// The lexicon and the allowlist, as the service has them; no judge and no
// hold log, since this is a script and --publish can post for real.
const safety = lexicalGate({ extraAllowedLinks: config.LINK_ALLOWLIST })
const logger = {
  info: (o: unknown, m?: string) => console.log('[info]', m ?? '', JSON.stringify(o)),
  warn: (o: unknown, m?: string) => console.warn('[warn]', m ?? '', JSON.stringify(o)),
  error: (o: unknown, m?: string) => console.error('[error]', m ?? '', o),
}

const job = new CalendarJob({
  agent: new ContentAgent({
    memory,
    x,
    ...(media ? { media } : {}),
    safety,
    botUsername: config.X_BOT_USERNAME,
    dryRun,
    llm: { baseUrl: config.LLM_BASE_URL, apiKey: config.LLM_API_KEY, model: config.LLM_MODEL },
    maxSteps: config.AGENT_MAX_STEPS,
    persona: loadPersona({ soulPath: config.SOUL_PATH, contextPath: config.CONTEXT_PATH, skillsDir: config.SKILLS_DIR }),
    ...(config.EXA_API_KEY ? { exaApiKey: config.EXA_API_KEY } : {}),
    ...(config.CHART_TOKEN_ADDRESS
      ? {
          market: {
            market: new TokenMarket({
              network: config.CHART_CHAIN,
              ...(config.SOLANA_RPC_URL ? { solanaRpcUrl: config.SOLANA_RPC_URL } : {}),
              watchlist: config.KOL_WALLETS,
            }),
            address: config.CHART_TOKEN_ADDRESS,
            symbol: config.CHART_TOKEN_SYMBOL,
            global: new GlobalMarketClient(),
          },
        }
      : {}),
    searchLimit: config.X_SEARCH_MAX_RESULTS,
    ownTokenAddresses: config.OWN_TOKEN_ADDRESSES,
  }),
  memory,
  x,
  ...(config.TELEGRAM_BOT_TOKEN ? { telegram: new TelegramApiClient({ token: config.TELEGRAM_BOT_TOKEN }) } : {}),
  ...(media ? { media: media.store } : {}),
  logger,
  dryRun,
  telegramChatIds: config.CALENDAR_TELEGRAM_CHAT_IDS,
  maxEmoji: config.MAX_EMOJI_PER_POST,
  safety,
})

console.log(`running "${item.name}" (${item.channels.join('+')}, media: ${item.media}) ${dryRun ? 'DRY' : 'FOR REAL'}`)
await job.run(item, { force: true })
for (const stored of media?.store.list() ?? []) console.log(`generated: ${stored.path ?? stored.id}`)
process.exit(0)
