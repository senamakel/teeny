import type { Config } from '../config.js'
import { NativeChartRenderer, ScreenshotChartRenderer, type ChartRenderer } from '../chart/renderer.js'
import { TokenMarket } from '../market/market.js'
import { ChartAgent } from './chart-agent.js'
import { captureChartPost, ChartHarness } from '../harness/chart-harness.js'
import type { Persona } from './soul.js'
import type { Memory, XTools } from '../types.js'

/**
 * Builds the chart agent from configuration.
 *
 * Shared by the service and the dry-run script on purpose: a rehearsal that
 * builds its own slightly different agent rehearses a different agent, and the
 * one thing a dry run has to be is the same as the real thing minus publishing.
 */
export function buildChartAgent(
  config: Config,
  options: {
    memory: Memory
    x: XTools
    botUsername: string
    persona?: Persona
    dryRun: boolean
    allowPosting?: boolean
  },
): ChartHarness {
  const address = config.CHART_TOKEN_ADDRESS
  if (!address) throw new Error('CHART_TOKEN_ADDRESS is not set, so there is no token to chart')

  const market = new TokenMarket({
    network: config.CHART_CHAIN,
    ...(config.SOLANA_RPC_URL ? { solanaRpcUrl: config.SOLANA_RPC_URL } : {}),
    ...(config.GMGN_BASE_URL ? { gmgnBaseUrl: config.GMGN_BASE_URL } : {}),
    watchlist: config.KOL_WALLETS,
  })

  const agent = new ChartAgent({
    memory: options.memory,
    market,
    charts: buildRenderer(config),
    x: options.x,
    address,
    symbol: config.CHART_TOKEN_SYMBOL,
    botUsername: options.botUsername,
    dryRun: options.dryRun,
    llm: {
      baseUrl: config.LLM_BASE_URL,
      apiKey: config.LLM_API_KEY,
      model: config.LLM_MODEL,
    },
    ...(options.persona ? { persona: options.persona } : {}),
    ...(config.EXA_API_KEY ? { exaApiKey: config.EXA_API_KEY } : {}),
    maxSteps: config.AGENT_MAX_STEPS,
    maxEmoji: config.MAX_EMOJI_PER_POST,
    searchLimit: config.X_SEARCH_MAX_RESULTS,
    watermark: config.CHART_WATERMARK ?? `@${options.botUsername}`,
    outputDir: config.CHART_OUTPUT_DIR,
    ...(options.allowPosting === false ? { allowPosting: false } : {}),
    ...(config.CHART_COMMUNITY_QUERY ? { communityQuery: config.CHART_COMMUNITY_QUERY } : {}),
    onPublished: (post) => captureChartPost(options.memory, config.CHART_TOKEN_SYMBOL, post),
  })

  // The agent does the work; the harness is what puts memory around it, on both
  // sides -- what to know before writing, and what to keep afterwards.
  return new ChartHarness(options.memory, agent, { symbol: config.CHART_TOKEN_SYMBOL })
}

export function buildRenderer(config: Config): ChartRenderer {
  if (config.CHART_RENDERER !== 'screenshot') return new NativeChartRenderer()
  return new ScreenshotChartRenderer({
    chain: config.CHART_CHAIN,
    ...(config.CHART_URL_TEMPLATE ? { urlTemplate: config.CHART_URL_TEMPLATE } : {}),
    ...(config.CHART_BROWSER_CHANNEL ? { channel: config.CHART_BROWSER_CHANNEL } : {}),
    ...(config.CHART_BROWSER_PATH ? { executablePath: config.CHART_BROWSER_PATH } : {}),
  })
}
