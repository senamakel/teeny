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

export type Timeframe = '5m' | '15m' | '1h' | '4h' | '1d'

/** How a timeframe is spelled for GeckoTerminal, which splits it in two. */
export const TIMEFRAMES: Record<Timeframe, { unit: 'minute' | 'hour' | 'day'; aggregate: number; seconds: number }> = {
  '5m': { unit: 'minute', aggregate: 5, seconds: 300 },
  '15m': { unit: 'minute', aggregate: 15, seconds: 900 },
  '1h': { unit: 'hour', aggregate: 1, seconds: 3_600 },
  '4h': { unit: 'hour', aggregate: 4, seconds: 14_400 },
  '1d': { unit: 'day', aggregate: 1, seconds: 86_400 },
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
