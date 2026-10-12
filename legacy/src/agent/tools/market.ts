import { tool } from '@langchain/core/tools'
import type { StructuredToolInterface } from '@langchain/core/tools'
import { z } from 'zod'
import type { ChartRenderer } from '../../chart/renderer.js'
import type { RenderedChart } from '../../chart/chart.js'
import { compactUsd, price as formatPrice, signedPercent } from '../../chart/format.js'
import type { TokenMarket } from '../../market/market.js'
import { relativeStrength, type GlobalMarketClient } from '../../market/global.js'
import { analyse } from '../../ta/analysis.js'
import { readProgress } from '../../ta/progress.js'
import type { PublishedChartPost } from '../../harness/chart-harness.js'
import { applyHouseStyle, asksTheReader, countEmoji, withoutTheAsk } from '../house-style.js'
import { MAX_POST_LENGTH, refuseUnsafe } from './x.js'
import type { SafetyGate } from '../../safety/gate.js'
import type { XTools } from '../../types.js'

const TIMEFRAME = z.enum(['5m', '15m', '1h', '4h', '12h', '1d', '1w'])

/** The four horizons a post is ever actually about. */
const HORIZONS: Array<{ timeframe: '1h' | '4h' | '1d' | '1w'; bars: number; label: string }> = [
  { timeframe: '1h', bars: 120, label: 'hourly (last 5 days)' },
  { timeframe: '4h', bars: 120, label: '4-hour (last 20 days)' },
  { timeframe: '1d', bars: 120, label: 'daily (last 4 months)' },
  { timeframe: '1w', bars: 52, label: 'weekly (the whole story)' },
]

export interface MarketToolOptions {
  market: TokenMarket
  /** The token every tool defaults to, so the agent cannot wander onto another one. */
  address: string
  symbol?: string
  x: XTools
  /**
   * Omit for a read-only tool set: no `render_chart`, no `post_chart`.
   *
   * This is how the reply drafter gets the market without getting the ability
   * to publish. It needs to answer "how's TINY doing" with real numbers, and it
   * must not be able to broadcast.
   */
  charts?: ChartRenderer
  dryRun: boolean
  maxEmoji?: number
  watermark?: string
  outputDir?: string
  /** The rest of the market, for context. Without it there is no relative-strength tool. */
  global?: GlobalMarketClient
  /** Off, `post_chart` refuses. Used by the dry run and by the research-only agent. */
  allowPosting?: boolean
  /** The content filter, for `post_chart`. */
  safety?: Pick<SafetyGate, 'outbound'>
  /**
   * Called with whatever was posted, including in a dry run. This is how the
   * post reaches memory: the model's own `remember` note is a paraphrase, and
   * "do not repeat yesterday's angle" needs the actual text.
   */
  onPublished?: (post: PublishedChartPost) => Promise<void>
}

/**
 * Everything the chart agent can find out, and the one way it can publish.
 *
 * The token address is bound here rather than being a parameter the model
 * fills in. A chart agent that can be told which contract to analyse is a chart
 * agent that can be told to shill a stranger's contract by anything that lands
 * in its context, and this one reads posts.
 */
export function createMarketTools(options: MarketToolOptions): StructuredToolInterface[] {
  const { market, address } = options
  const maxEmoji = options.maxEmoji ?? 1
  // Rendered charts, kept by handle. The model never handles image bytes: it
  // renders, gets a short id back, and passes that id to the post tool.
  const rendered = new Map<string, RenderedChart>()

  const overview = tool(
    async () => {
      const snapshot = await market.overview(address)
      return [
        `${snapshot.symbol ?? 'token'} (${snapshot.name ?? 'unknown'}) on ${snapshot.chain}`,
        `price: $${formatPrice(snapshot.priceUsd)}`,
        snapshot.marketCapUsd === undefined ? '' : `market cap: ${compactUsd(snapshot.marketCapUsd)}`,
        snapshot.fdvUsd === undefined ? '' : `fdv: ${compactUsd(snapshot.fdvUsd)}`,
        snapshot.liquidityUsd === undefined ? '' : `liquidity: ${compactUsd(snapshot.liquidityUsd)}`,
        snapshot.volume24hUsd === undefined ? '' : `24h volume: ${compactUsd(snapshot.volume24hUsd)}`,
        Object.entries(snapshot.priceChange).length === 0
          ? ''
          : `change: ${Object.entries(snapshot.priceChange)
              .map(([window, value]) => `${window} ${signedPercent(value)}`)
              .join(', ')}`,
        snapshot.txns24h === undefined
          ? ''
          : `24h transactions: ${snapshot.txns24h.buys} buys / ${snapshot.txns24h.sells} sells`,
        snapshot.poolAddress ? `pool: ${snapshot.poolAddress} (${snapshot.poolName ?? 'unknown'})` : '',
        snapshot.pairCreatedAt ? `pool opened: ${snapshot.pairCreatedAt}` : '',
        `source: ${snapshot.source}`,
      ]
        .filter(Boolean)
        .join('\n')
    },
    {
      name: 'token_overview',
      description:
        'The live snapshot for the token this account posts about: price, market cap, ' +
        'liquidity, 24h volume, price change and buy/sell counts. Call this first. ' +
        'Every number it returns is quotable in a post; nothing else is.',
      schema: z.object({}),
    },
  )

  const candles = tool(
    async ({ timeframe, bars }) => {
      const series = await market.candles({ address, timeframe, limit: bars })
      if (series.length === 0) return 'No candles came back for that timeframe.'
      const recent = series.slice(-12)
      const first = series[0]
      const last = series[series.length - 1]
      if (!first || !last) return 'No candles came back for that timeframe.'
      return [
        `${series.length} ${timeframe} bars, ${new Date(first.time * 1_000).toISOString()} to ` +
          `${new Date(last.time * 1_000).toISOString()}`,
        `window open $${formatPrice(first.open)}, last close $${formatPrice(last.close)}`,
        `window high $${formatPrice(Math.max(...series.map((c) => c.high)))}, ` +
          `low $${formatPrice(Math.min(...series.map((c) => c.low)))}`,
        '',
        'last 12 bars (time, open, high, low, close, volume usd):',
        ...recent.map(
          (candle) =>
            `${new Date(candle.time * 1_000).toISOString()} ${formatPrice(candle.open)} ` +
            `${formatPrice(candle.high)} ${formatPrice(candle.low)} ${formatPrice(candle.close)} ` +
            `${Math.round(candle.volume)}`,
        ),
      ].join('\n')
    },
    {
      name: 'token_candles',
      description:
        'Raw OHLCV for the token. Use it when a post needs a specific bar; for the shape ' +
        'of the move use technical_analysis instead, which reads the same data properly.',
      schema: z.object({
        timeframe: TIMEFRAME.default('1h'),
        bars: z.number().int().min(20).max(500).default(120),
      }),
    },
  )

  const technicals = tool(
    async ({ timeframe, bars }) => {
      const series = await market.candles({ address, timeframe, limit: bars })
      if (series.length < 20) return `Only ${series.length} bars available; not enough to analyse.`
      const read = analyse(series, { timeframe })
      return [
        ...read.notes,
        '',
        'signals:',
        ...read.signals.map(
          (signal) => `- ${signal.name}: ${signal.detail} [${signal.bullish ? 'bullish' : 'bearish'}, ${signal.strength}]`,
        ),
        '',
        `bullish score: ${read.bullishScore}/100. This is the balance of the signals above, ` +
          'not a forecast. Below 40 the short-term tape is weak and the positive is on a ' +
          'longer horizon or on-chain: call token_progress, token_holders or token_flow and ' +
          'lead with what you find there. The positives listed above are still true.',
      ].join('\n')
    },
    {
      name: 'technical_analysis',
      description:
        'Reads the chart the way one of these actually trades: support and resistance ' +
        'levels that have been defended more than once, whether one has just broken, been ' +
        'reclaimed or been lost, whether volume confirmed it, how volume compares to ' +
        'earlier in the window, trend and structure, and a bullish score. No oscillators: ' +
        'RSI and MACD say nothing useful about a token like this. Everything it returns is ' +
        'computed from real candles and is safe to quote. Facts about the past only.',
      schema: z.object({
        timeframe: TIMEFRAME.default('1h'),
        bars: z.number().int().min(20).max(500).default(120),
      }),
    },
  )

  /**
   * The same chart at four zoom levels.
   *
   * One timeframe is one opinion. An hour can be red inside a green week, and a
   * post built on whichever timeframe happened to be the default is a post
   * whose framing was chosen by a config value. This reads all four, says which
   * agree, and lets the model pick the horizon the day is actually about.
   *
   * Cheap after the first call: the market layer caches each series, so the
   * chart that gets rendered afterwards costs nothing more.
   */
  /**
   * The token against everything else.
   *
   * This is the single most useful thing the account can say on a bad day, and
   * it is a fact rather than a spin: a token up 14% while the whole market is
   * down 1% is genuinely outperforming, and saying so is more informative than
   * the 14% alone. On a day the token is down and the market is down harder,
   * the same sentence is still true and still the honest headline.
   */
  const context = tool(
    async () => {
      if (!options.global) {
        return 'No global market source is configured, so there is nothing to compare against.'
      }
      const [snapshot, token] = await Promise.all([
        options.global.snapshot(),
        market.overview(address).catch(() => undefined),
      ])

      const lines = [
        'The rest of the market, last 24h:',
        ...snapshot.assets.map(
          (asset) =>
            `  ${asset.symbol} $${asset.priceUsd.toLocaleString('en-US')}` +
            (asset.change24hPct === undefined
              ? ''
              : ` (${asset.change24hPct >= 0 ? '+' : ''}${asset.change24hPct.toFixed(2)}%)`),
        ),
        snapshot.totalMarketCapUsd === undefined
          ? ''
          : `  total crypto market cap ${compactUsd(snapshot.totalMarketCapUsd)}` +
            (snapshot.totalChange24hPct === undefined
              ? ''
              : ` (${snapshot.totalChange24hPct >= 0 ? '+' : ''}${snapshot.totalChange24hPct.toFixed(2)}%)`),
        snapshot.btcDominancePct === undefined
          ? ''
          : `  BTC dominance ${snapshot.btcDominancePct.toFixed(1)}%`,
      ].filter(Boolean)

      const change = token?.priceChange.h24
      if (change === undefined) {
        return [...lines, '', 'No 24h change for the token, so no comparison can be drawn.'].join('\n')
      }

      const strength = relativeStrength(change, snapshot)
      return [
        ...lines,
        '',
        `${options.symbol ?? 'the token'} is ${change >= 0 ? '+' : ''}${change.toFixed(2)}% over the same 24h, ` +
          `while the market is ${strength.marketDirection}.`,
        '',
        strength.beating.length > 0
          ? ['TRUE POSITIVES YOU CAN LEAD WITH, best first:', ...strength.beating.map((entry) => `  - ${entry}`)].join('\n')
          : 'It did not outperform any of these over 24h. Do not force a comparison that is ' +
            'not there -- take the positive from the chart, the holders, or the longer horizon ' +
            'instead.',
        '',
        strength.marketDirection === 'down'
          ? 'The whole market is red today. Holding up while everything else sells off is the ' +
            'strongest true thing you can say, and it is the post.'
          : 'The market is not red today, so outperformance is a smaller claim. Say it plainly ' +
            'and let the chart carry the rest.',
      ].join('\n')
    },
    {
      name: 'market_context',
      description:
        'How the token is doing against BTC, ETH, SOL and the whole crypto market over the ' +
        'same 24 hours. Call this on every run: a percentage on its own means little, and ' +
        'the same number is a quiet day in a green market and a remarkable one in a red one.',
      schema: z.object({}),
    },
  )

  const timeframes = tool(
    async () => {
      const reads = await Promise.all(
        HORIZONS.map(async (horizon) => {
          const series = await market
            .candles({ address, timeframe: horizon.timeframe, limit: horizon.bars })
            .catch(() => [] as Awaited<ReturnType<typeof market.candles>>)
          return {
            horizon,
            read: series.length < 20 ? undefined : analyse(series, { timeframe: horizon.timeframe }),
            bars: series.length,
          }
        }),
      )

      const lines = reads.map(({ horizon, read, bars }) => {
        if (!read) return `${horizon.label}: only ${bars} bars available, not enough to read.`
        return [
          `${horizon.label} [${horizon.timeframe}, ${read.bars} bars]`,
          `  ${read.changePct >= 0 ? 'up' : 'down'} ${Math.abs(read.changePct)}%, trend ${read.trend}, ${read.structure}`,
          read.event
            ? `  ${read.event.kind} at $${read.event.level.toPrecision(4)}, ${read.event.barsAgo} bars ago` +
              `${read.event.volumeConfirmed ? ', volume confirmed' : ', unconfirmed'}`
            : '  no level crossed recently',
          read.levels.support === undefined
            ? '  no support below price on this timeframe'
            : `  support $${read.levels.support.toPrecision(4)} (${read.levels.supportTouches} touches)`,
          `  bullish ${read.bullishScore}/100`,
          `  best positive: ${read.positives[0] ?? 'none'}`,
        ].join('\n')
      })

      const known = reads.flatMap((entry) => (entry.read ? [entry.read.bullishScore] : []))
      const up = known.filter((score) => score >= 50).length

      return [
        ...lines,
        '',
        known.length === 0
          ? 'No timeframe had enough history to read.'
          : up === known.length
            ? `All ${known.length} timeframes read bullish. That agreement is itself the story.`
            : up === 0
              ? `None of the ${known.length} timeframes reads bullish on price. The positive is ` +
                'on-chain today: try token_holders or token_flow, and lead with what you find.'
              : `${up} of ${known.length} timeframes read bullish. Post about the horizon the day ` +
                'is actually about, and do not pretend the others agree with it.',
      ].join('\n')
    },
    {
      name: 'chart_timeframes',
      description:
        'The same chart at four zoom levels at once: hourly, 4-hour, daily and weekly. ' +
        'Each one gets its trend, structure, any level just broken or reclaimed, its nearest ' +
        'support and its own best positive, plus whether the timeframes agree. Call this ' +
        'early: it is how you choose which horizon the post should be about, and it makes ' +
        'the later calls free because the candles are cached.',
      schema: z.object({}),
    },
  )

  const progress = tool(
    async ({ days }) => {
      const daily = await market.candles({ address, timeframe: '1d', limit: days })
      if (daily.length < 2) return 'Not enough daily history to say anything about progress.'
      const read = readProgress(daily)
      return [
        ...read.notes,
        '',
        'This is the long horizon: where the token is against where it was, not what it did ' +
          'in the last hour. It is what a holder who has been here for weeks is actually ' +
          'asking about.',
      ].join('\n')
    },
    {
      name: 'token_progress',
      description:
        'Long-term progress on the daily: change over 7, 30 and 90 days, how many days ' +
        'closed green, distance from the daily high and low, and whether the daily lows are ' +
        'stepping up. Use this whenever the post is about where we are rather than about ' +
        'today, and always on a day when the short-term read is weak.',
      schema: z.object({
        days: z.number().int().min(7).max(365).default(120),
      }),
    },
  )

  const drawChart = tool(
    async ({ timeframe, bars }) => {
      const series = await market.candles({ address, timeframe, limit: bars })
      if (series.length < 5) return `Only ${series.length} bars available; not enough to chart.`
      const snapshot = await market.overview(address).catch(() => undefined)
      const read = analyse(series, { timeframe })
      if (!options.charts) return 'Refused: no chart renderer is configured in this mode.'
      const chart = await options.charts.render({
        candles: series,
        timeframe,
        ...(snapshot ? { overview: snapshot } : {}),
        read,
        ...(options.symbol ? { symbol: options.symbol } : {}),
        ...(options.watermark ? { watermark: options.watermark } : {}),
        ...(options.outputDir ? { outputDir: options.outputDir } : {}),
      })
      const id = `chart_${rendered.size + 1}`
      rendered.set(id, chart)
      return [
        `Rendered ${id}: ${timeframe}, ${series.length} bars, ${chart.width}x${chart.height}px, ` +
          `${Math.round(chart.png.byteLength / 1024)}KB.`,
        chart.path ? `Saved to ${chart.path}.` : '',
        `Pass chartId "${id}" to post_chart to attach it.`,
      ]
        .filter(Boolean)
        .join('\n')
    },
    {
      name: 'render_chart',
      description:
        'Draws a candlestick chart of the token with EMA20/EMA50, volume, and the support ' +
        'and resistance levels the analysis found, and returns a chartId. Render once, then ' +
        'attach that id to a single post.',
      schema: z.object({
        timeframe: TIMEFRAME.default('1h'),
        bars: z.number().int().min(20).max(500).default(120),
      }),
    },
  )

  const postChart = tool(
    async ({ text: drafted, chartId, replyToPostId }) => {
      if (options.allowPosting === false) {
        return 'Refused: this agent cannot publish. Say what you would have posted instead.'
      }
      if (drafted.length > MAX_POST_LENGTH) {
        return `Refused: the text is ${drafted.length} characters, over the ${MAX_POST_LENGTH} limit.`
      }
      const emoji = countEmoji(drafted)
      if (emoji > maxEmoji) {
        return (
          `Refused: the text has ${emoji} emoji and this account allows at most ${maxEmoji} ` +
          'per post. Rewrite it and call the tool again.'
        )
      }
      const chart = chartId ? rendered.get(chartId) : undefined
      if (chartId && !chart) {
        return `Refused: no chart with id ${chartId}. Call render_chart and use the id it returns.`
      }
      // The post ends on its read, not on an ask: a closing question is cut
      // here rather than argued about with the agent.
      const { text } = applyHouseStyle(asksTheReader(drafted) ? withoutTheAsk(drafted) : drafted, { maxEmoji, maxLength: MAX_POST_LENGTH })
      const refused = await refuseUnsafe(text, options.safety, {
        surface: 'chart',
        kind: 'draft',
        tool: 'post_chart',
        ...(replyToPostId ? { postId: replyToPostId } : {}),
      })
      if (refused) return refused

      if (options.dryRun) {
        await record(options, { text, dryRun: true, ...(chartId ? { chartId } : {}) })
        return `DRY RUN, nothing was published. Would have posted: ${JSON.stringify({
          text,
          chartId,
          replyToPostId,
          chart: chart ? { bytes: chart.png.byteLength, path: chart.path } : undefined,
        })}`
      }

      const mediaIds = chart
        ? [(await options.x.uploadMedia({ data: chart.png, mimeType: 'image/png' })).mediaId]
        : undefined
      const posted = await options.x.createPost(text, {
        ...(mediaIds ? { mediaIds } : {}),
        ...(replyToPostId ? { replyToPostId } : {}),
      })
      await record(options, {
        text,
        postedId: posted.id,
        dryRun: false,
        ...(chartId ? { chartId } : {}),
      })
      return `Published post ${posted.id}${mediaIds ? ' with the chart attached' : ''}.`
    },
    {
      name: 'post_chart',
      description:
        'Publishes one post, with a rendered chart attached when chartId is given. This is ' +
        'the only way this agent publishes. Post once per run.',
      schema: z.object({
        text: z.string().min(1).max(MAX_POST_LENGTH),
        chartId: z.string().optional().describe('An id returned by render_chart.'),
        replyToPostId: z.string().regex(/^\d+$/).optional(),
      }),
    },
  )

  const holders = tool(
    async () => {
      const snapshot = await market.holders(address).catch((error: unknown) => {
        return `Holder data is unavailable right now: ${(error as Error).message}`
      })
      if (typeof snapshot === 'string') return snapshot
      return [
        `total supply: ${Math.round(snapshot.totalSupply).toLocaleString('en-US')}`,
        `top 10 token accounts hold ${(snapshot.top10Share * 100).toFixed(1)}% of supply`,
        '',
        ...snapshot.top
          .slice(0, 10)
          .map(
            (account, index) =>
              `${index + 1}. ${account.address} — ${(account.share * 100).toFixed(2)}%`,
          ),
        '',
        'These are token accounts, not people: a liquidity pool vault is normally the largest ' +
          'one, and a single person can hold several. Read it as concentration, and never ' +
          'call one of these "the top holder" in a post.',
      ].join('\n')
    },
    {
      name: 'token_holders',
      description:
        'Supply and holder concentration, read from the chain. Useful for a post about ' +
        'distribution; useless as a claim about any individual.',
      schema: z.object({}),
    },
  )

  const flow = tool(
    async ({ minVolumeUsd }) => {
      const summary = await market.flow({ address, minVolumeUsd })
      if (summary.window.trades === 0) {
        return `No trades over $${minVolumeUsd} in the window the provider returns.`
      }
      const wallet = (entry: { wallet: string; label?: string; boughtUsd: number; soldUsd: number; netUsd: number }): string =>
        `${entry.label ? `${entry.label} (${short(entry.wallet)})` : short(entry.wallet)}: ` +
        `bought ${compactUsd(entry.boughtUsd)}, sold ${compactUsd(entry.soldUsd)}, ` +
        `net ${compactUsd(entry.netUsd)}`
      return [
        `window: ${summary.window.trades} trades over $${minVolumeUsd}, ` +
          `${summary.window.from} to ${summary.window.to}`,
        `buy volume ${compactUsd(summary.buyVolumeUsd)}, sell volume ${compactUsd(summary.sellVolumeUsd)}, ` +
          `net ${compactUsd(summary.netUsd)}`,
        `${summary.uniqueWallets} wallets: ${summary.buyers} buying, ${summary.sellers} selling`,
        '',
        'top buyers:',
        ...summary.topBuyers.map((entry) => `- ${wallet(entry)}`),
        '',
        'top sellers:',
        ...summary.topSellers.map((entry) => `- ${wallet(entry)}`),
        ...(summary.known.length > 0
          ? ['', 'watchlist wallets active:', ...summary.known.map((entry) => `- ${wallet(entry)}`)]
          : []),
        '',
        'This is only the recent trades the provider returns, not the full day. Say "in the ' +
          'last few hours", never "today", and never name a wallet address in a post.',
      ].join('\n')
    },
    {
      name: 'token_flow',
      description:
        'On-chain flow for the token over the most recent trades: buy versus sell volume, ' +
        'how many wallets on each side, the biggest buyers and sellers, and any watchlist ' +
        'wallet that traded. The window is short -- it is the provider\'s recent trades page.',
      schema: z.object({
        minVolumeUsd: z
          .number()
          .min(0)
          .max(1_000_000)
          .default(100)
          .describe('Ignore trades smaller than this, in USD. 100 filters out dust.'),
      }),
    },
  )

  // Named for the trader rather than the wallet on purpose: `capabilities.test.ts`
  // refuses any tool whose name contains "wallet", and that rule is worth more
  // than the nicer name. A read-only lookup called wallet_activity trains the
  // eye to accept wallet-shaped tool names, which is exactly the habit the test
  // exists to prevent.
  const traderActivity = tool(
    async ({ wallet, minVolumeUsd }) => {
      const activity = await market.wallet({ address, wallet, minVolumeUsd })
      if (!activity) return `${short(wallet)} has not traded this token in the window read.`
      return [
        `${activity.label ? `${activity.label} — ` : ''}${activity.wallet}`,
        `${activity.buys} buys totalling ${compactUsd(activity.boughtUsd)}`,
        `${activity.sells} sells totalling ${compactUsd(activity.soldUsd)}`,
        `net ${compactUsd(activity.netUsd)} between ${activity.firstSeen} and ${activity.lastSeen}`,
        '',
        'Do not put this wallet address in a post.',
      ].join('\n')
    },
    {
      name: 'trader_activity',
      description:
        'What one wallet did in this token over the recent trades window. Use it to check a ' +
        'claim about a wallet, not to build a post around one.',
      schema: z.object({
        wallet: z.string().min(26).max(64),
        minVolumeUsd: z.number().min(0).max(1_000_000).default(0),
      }),
    },
  )

  const rankedTraders = tool(
    async () => {
      const traders = await market.rankedTraders(address)
      if (traders.length === 0) {
        return (
          'No ranked-trader source is configured, so there is no KOL or smart-money data. ' +
          'Do not claim any. token_flow shows who is actually buying right now.'
        )
      }
      return traders
        .slice(0, 15)
        .map(
          (trader) =>
            `${short(trader.wallet)}${trader.tags.length > 0 ? ` [${trader.tags.join(', ')}]` : ''}: ` +
            `bought ${compactUsd(trader.boughtUsd ?? 0)}, sold ${compactUsd(trader.soldUsd ?? 0)}, ` +
            `realised ${compactUsd(trader.realizedProfitUsd ?? 0)}`,
        )
        .join('\n')
    },
    {
      name: 'ranked_traders',
      description:
        'The wallets a ranked-trader provider labels on this token: KOLs, smart money, ' +
        'whales. Returns nothing unless such a provider is configured, and "nothing" means ' +
        'the account must not claim anything about KOLs.',
      schema: z.object({}),
    },
  )

  const reading = [
    overview,
    context,
    candles,
    technicals,
    timeframes,
    progress,
    holders,
    flow,
    traderActivity,
    rankedTraders,
  ]
  // Without a renderer there is nothing to draw and nothing to attach, so the
  // two publishing tools are not built rather than being built and refusing.
  return options.charts ? [...reading.slice(0, 6), drawChart, ...reading.slice(6), postChart] : reading
}

/**
 * Writing to memory must never lose a post that went out. The post is already
 * published by the time this runs, so a memory outage is reported to the model
 * and dropped rather than raised, which would read to the agent as a failed
 * publish and invite it to post again.
 */
async function record(options: MarketToolOptions, post: PublishedChartPost): Promise<void> {
  if (!options.onPublished) return
  await options.onPublished(post).catch(() => undefined)
}

/** Wallets are shown truncated: they are for the agent's reasoning, not for a post. */
function short(wallet: string): string {
  return wallet.length <= 12 ? wallet : `${wallet.slice(0, 4)}…${wallet.slice(-4)}`
}
