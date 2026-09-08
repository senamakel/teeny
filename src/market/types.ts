/**
 * Market data, kept deliberately small.
 *
 * Every field here is something a chart or a piece of technical analysis
 * actually consumes. The upstream APIs return a great deal more; carrying it
 * through would mean the difference between two providers showing up as a type
 * error in the renderer rather than in the adapter that knows about it.
 */

/** One OHLCV bar. `time` is unix seconds at the bar's open. */
export interface Candle {
  time: number
  open: number
  high: number
  low: number
  close: number
  volume: number
}

export type Timeframe = '5m' | '15m' | '1h' | '4h' | '12h' | '1d' | '1w'

export interface TimeframeSpec {
  unit: 'minute' | 'hour' | 'day'
  aggregate: number
  seconds: number
  /**
   * Fetch this timeframe instead and fold `factor` bars into one.
   *
   * Weekly candles do not exist upstream: GeckoTerminal's daily endpoint
   * accepts `aggregate=1` and nothing else, and answers 400 to anything larger.
   * Folding seven daily bars into one is the same arithmetic the exchange would
   * do, so the weekly here is real rather than approximate -- but it is built
   * from whatever daily history exists, which is why a young token's weekly is
   * a handful of bars rather than an error.
   */
  foldFrom?: { timeframe: Timeframe; factor: number }
}

/** How a timeframe is spelled for GeckoTerminal, which splits it in two. */
export const TIMEFRAMES: Record<Timeframe, TimeframeSpec> = {
  '5m': { unit: 'minute', aggregate: 5, seconds: 300 },
  '15m': { unit: 'minute', aggregate: 15, seconds: 900 },
  '1h': { unit: 'hour', aggregate: 1, seconds: 3_600 },
  '4h': { unit: 'hour', aggregate: 4, seconds: 14_400 },
  '12h': { unit: 'hour', aggregate: 12, seconds: 43_200 },
  '1d': { unit: 'day', aggregate: 1, seconds: 86_400 },
  '1w': {
    unit: 'day',
    aggregate: 1,
    seconds: 604_800,
    foldFrom: { timeframe: '1d', factor: 7 },
  },
}

/**
 * Folds a series into longer bars: open of the first, close of the last, the
 * extremes of the whole bucket, and volume summed.
 *
 * Bucketed on absolute time rather than by counting backwards from the newest
 * bar, so a weekly candle covers the same seven days whenever it is asked for.
 * Counting back from "now" would silently redraw every bar each time the run
 * happened at a different hour.
 */
export function fold(candles: Candle[], seconds: number): Candle[] {
  const buckets = new Map<number, Candle[]>()
  for (const candle of candles) {
    const key = Math.floor(candle.time / seconds) * seconds
    buckets.set(key, [...(buckets.get(key) ?? []), candle])
  }

  return [...buckets.entries()]
    .sort(([a], [b]) => a - b)
    .flatMap(([time, group]) => {
      const first = group[0]
      const last = group[group.length - 1]
      if (!first || !last) return []
      return [
        {
          time,
          open: first.open,
          close: last.close,
          high: Math.max(...group.map((candle) => candle.high)),
          low: Math.min(...group.map((candle) => candle.low)),
          volume: group.reduce((total, candle) => total + candle.volume, 0),
        },
      ]
    })
}

export interface PriceChange {
  m5?: number
  h1?: number
  h6?: number
  h24?: number
}

/** A token as the market sees it right now. */
export interface TokenOverview {
  address: string
  chain: string
  name?: string
  symbol?: string
  priceUsd: number
  marketCapUsd?: number
  fdvUsd?: number
  liquidityUsd?: number
  volume24hUsd?: number
  priceChange: PriceChange
  txns24h?: { buys: number; sells: number }
  /** The deepest pool, which is the one worth charting. */
  poolAddress?: string
  poolName?: string
  pairCreatedAt?: string
  /** Which provider answered, so a post can never cite a number without one. */
  source: string
}

/** A single swap, with the wallet that made it. */
export interface Trade {
  txHash: string
  timestamp: string
  kind: 'buy' | 'sell'
  wallet: string
  volumeUsd: number
  tokenAmount: number
  priceUsd: number
}

export interface HolderAccount {
  address: string
  amount: number
  /** Share of circulating supply, 0-1. */
  share: number
}

export interface HolderSnapshot {
  totalSupply: number
  decimals: number
  /** Largest token accounts, biggest first. These are token accounts, not owners. */
  top: HolderAccount[]
  top10Share: number
  source: string
}

/** What a market provider has to be able to answer. */
export interface MarketSource {
  overview(address: string): Promise<TokenOverview>
  candles(input: { pool: string; timeframe: Timeframe; limit?: number }): Promise<Candle[]>
  trades(input: { pool: string; minVolumeUsd?: number }): Promise<Trade[]>
}
