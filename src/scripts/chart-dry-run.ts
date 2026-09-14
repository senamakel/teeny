import 'dotenv/config'
import { buildChartAgent } from '../agent/chart-wiring.js'
import { AlgoCritic } from '../critic/algo.js'
import { loadPersona } from '../agent/soul.js'
import { XApiClient } from '../clients/x.js'
import { loadConfig } from '../config.js'
import { CortexMemory } from '../memory/cortex.js'

/**
 * Rehearses the daily chart post without publishing.
 *
 *   npm run chart-dry-run                 # the whole agent, dry
 *   npm run chart-dry-run -- --data-only  # just the market read and the chart
 *   npm run chart-dry-run -- --publish    # actually posts, if DRY_RUN=false
 *
 * The chart PNG is written to CHART_OUTPUT_DIR either way, so the image can be
 * looked at before anybody decides to let this thing post on a timer.
 */

const config = loadConfig()
const argv = process.argv.slice(2)
const dataOnly = argv.includes('--data-only')
const publish = argv.includes('--publish')
const dryRun = !publish || config.DRY_RUN

if (!config.CHART_TOKEN_ADDRESS) {
  console.error('CHART_TOKEN_ADDRESS is not set. Nothing to chart.')
  process.exit(1)
}

if (dataOnly) {
  const { TokenMarket } = await import('../market/market.js')
  const { analyse } = await import('../ta/analysis.js')
  const { buildRenderer } = await import('../agent/chart-wiring.js')

  const market = new TokenMarket({
    network: config.CHART_CHAIN,
    ...(config.SOLANA_RPC_URL ? { solanaRpcUrl: config.SOLANA_RPC_URL } : {}),
    ...(config.GMGN_BASE_URL ? { gmgnBaseUrl: config.GMGN_BASE_URL } : {}),
    watchlist: config.KOL_WALLETS,
  })
  const address = config.CHART_TOKEN_ADDRESS
  const overview = await market.overview(address)
  const candles = await market.candles({
    address,
    timeframe: config.CHART_TIMEFRAME,
    limit: config.CHART_BARS,
  })
  const read = analyse(candles, { timeframe: config.CHART_TIMEFRAME })

  console.log('\nOVERVIEW')
  console.log(overview)
  console.log('\nTECHNICAL READ')
  for (const note of read.notes) console.log(`  ${note}`)
  console.log('\nSIGNALS')
  for (const signal of read.signals) {
    console.log(
      `  ${signal.bullish ? '+' : '-'} ${signal.name} (${signal.strength}): ${signal.detail}`,
    )
  }
  console.log(`\nBULLISH SCORE: ${read.bullishScore}/100`)

  const chart = await buildRenderer(config).render({
    candles,
    timeframe: config.CHART_TIMEFRAME,
    overview,
    read,
    symbol: config.CHART_TOKEN_SYMBOL,
    watermark: config.CHART_WATERMARK ?? `@${config.X_BOT_USERNAME}`,
    outputDir: config.CHART_OUTPUT_DIR,
  })
  console.log(`\nCHART: ${chart.path ?? '(not saved)'} (${Math.round(chart.png.byteLength / 1024)}KB)`)

  const flow = await market.flow({ address, minVolumeUsd: 100 }).catch((error: unknown) => {
    console.log(`\nFLOW unavailable: ${(error as Error).message}`)
    return undefined
  })
  if (flow) {
    console.log('\nFLOW')
    console.log(
      `  ${flow.window.trades} trades, buy ${Math.round(flow.buyVolumeUsd)} / sell ` +
        `${Math.round(flow.sellVolumeUsd)}, net ${Math.round(flow.netUsd)}, ` +
        `${flow.uniqueWallets} wallets`,
    )
  }
  process.exit(0)
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
// From the token, so a rehearsal signs its watermark with the handle the post
// would actually go out under.
const botUsername = await x
  .identity()
  .then((identity) => identity.username)
  .catch(() => config.X_BOT_USERNAME)

const agent = buildChartAgent(config, {
  memory,
  x,
  botUsername,
  persona: loadPersona({
    soulPath: config.SOUL_PATH,
    contextPath: config.CONTEXT_PATH,
    skillsDir: config.SKILLS_DIR,
  }),
  dryRun,
  ...(dryRun ? { allowPosting: false } : {}),
  // The critic as the service has it, so a rehearsal shows the ranker's read
  // of the post text and the rewrite it asked for.
  ...(config.ALGO_CRITIC
    ? { critic: new AlgoCritic({ baseUrl: config.LLM_BASE_URL, apiKey: config.LLM_API_KEY, model: config.ALGO_CRITIC_MODEL }) }
    : {}),
})

console.log(
  `\nRunning the chart agent for ${config.CHART_TOKEN_SYMBOL} as @${botUsername}. ` +
    `${dryRun ? 'Nothing will be published.' : 'THIS WILL PUBLISH.'}\n`,
)

const result = await agent.run({
  task:
    `Post today's ${config.CHART_TOKEN_SYMBOL} chart. Start on the ${config.CHART_TIMEFRAME} ` +
    'timeframe. Follow your instructions exactly: research, render once, post once, remember it.',
  threadId: `chart-dry-run:${Date.now()}`,
})

console.log('TOOLS CALLED')
for (const step of result.steps) console.log(`  ${step.tool} ${JSON.stringify(step.input)}`)
console.log('\nAGENT OUTPUT\n')
console.log(result.output)
console.log()
