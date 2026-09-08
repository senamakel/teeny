import { fetchJson, num } from './http.js'

export interface Asset {
  symbol: string
  priceUsd: number
  change24hPct?: number
}

export interface GlobalSnapshot {
  totalMarketCapUsd?: number
  totalChange24hPct?: number
  btcDominancePct?: number
  assets: Asset[]
  source: string
}

/** How the token did against the rest of the market over the same 24 hours. */
export interface RelativeStrength {
  /** Percentage points of outperformance. Positive is beating it. */
  against: Array<{ what: string; points: number; theirChangePct: number }>
  /** The comparisons that flatter, best first. Empty when the token lagged everything. */
  beating: string[]
  marketDirection: 'up' | 'down' | 'flat'
}

const BASE = 'https://api.coingecko.com/api/v3'

/**
 * The rest of the market, for context.
 *
 * A number on its own says very little: 14% is a quiet day in a bull run and a
 * remarkable one on a day the whole market is red. This is what lets a post say
 * the second thing, which is both truer and more useful to somebody holding
 * through it than the raw percentage on its own.
 */
export class GlobalMarketClient {
  constructor(private readonly options: { baseUrl?: string; timeoutMs?: number } = {}) {}

  async snapshot(): Promise<GlobalSnapshot> {
    const base = this.options.baseUrl ?? BASE
    const timeout = this.options.timeoutMs === undefined ? {} : { timeoutMs: this.options.timeoutMs }

    const [global, prices] = await Promise.all([
      fetchJson<{ data?: Record<string, unknown> }>(`${base}/global`, timeout).catch(() => undefined),
      fetchJson<Record<string, Record<string, unknown>>>(
        `${base}/simple/price?ids=bitcoin,solana,ethereum&vs_currencies=usd&include_24hr_change=true`,
        timeout,
      ).catch(() => undefined),
    ])
    if (!global && !prices) throw new Error('No global market data is available right now')

    const data = global?.data ?? {}
    const caps = (data['total_market_cap'] ?? {}) as Record<string, unknown>
    const dominance = (data['market_cap_percentage'] ?? {}) as Record<string, unknown>

    const assets: Asset[] = [
      ['bitcoin', 'BTC'],
      ['ethereum', 'ETH'],
      ['solana', 'SOL'],
    ].flatMap(([id, symbol]) => {
      const entry = prices?.[id as string]
      const price = num(entry?.['usd'])
      if (price === undefined) return []
      return [
        {
          symbol: symbol as string,
          priceUsd: price,
          ...(num(entry?.['usd_24h_change']) === undefined
            ? {}
            : { change24hPct: num(entry?.['usd_24h_change']) as number }),
        },
      ]
    })

    return {
      ...(num(caps['usd']) === undefined ? {} : { totalMarketCapUsd: num(caps['usd']) as number }),
      ...(num(data['market_cap_change_percentage_24h_usd']) === undefined
        ? {}
        : { totalChange24hPct: num(data['market_cap_change_percentage_24h_usd']) as number }),
      ...(num(dominance['btc']) === undefined ? {} : { btcDominancePct: num(dominance['btc']) as number }),
      assets,
      source: 'coingecko',
    }
  }
}

/**
 * The token against the market, in percentage points.
 *
 * Points rather than ratios on purpose: "+15 points on SOL" is a sentence a
 * person can read, where "1.14x SOL's return" is one they have to decode. And
 * comparisons are only kept when the token actually won them -- a post is
 * allowed to choose which true comparison to make, and there is no reason to
 * hand the model a list of the ones where it lost.
 */
export function relativeStrength(
  tokenChange24hPct: number,
  snapshot: GlobalSnapshot,
): RelativeStrength {
  const against = [
    ...snapshot.assets.flatMap((asset) =>
      asset.change24hPct === undefined
        ? []
        : [
            {
              what: asset.symbol,
              points: round(tokenChange24hPct - asset.change24hPct),
              theirChangePct: round(asset.change24hPct),
            },
          ],
    ),
    ...(snapshot.totalChange24hPct === undefined
      ? []
      : [
          {
            what: 'the whole crypto market',
            points: round(tokenChange24hPct - snapshot.totalChange24hPct),
            theirChangePct: round(snapshot.totalChange24hPct),
          },
        ]),
  ]

  const direction = snapshot.totalChange24hPct ?? snapshot.assets[0]?.change24hPct ?? 0
  return {
    against,
    beating: against
      .filter((entry) => entry.points > 0)
      .sort((a, b) => b.points - a.points)
      .map(
        (entry) =>
          `outperforming ${entry.what} by ${entry.points} points over 24h ` +
          `(${entry.what} ${entry.theirChangePct >= 0 ? '+' : ''}${entry.theirChangePct}%)`,
      ),
    marketDirection: direction > 0.5 ? 'up' : direction < -0.5 ? 'down' : 'flat',
  }
}

function round(value: number, places = 1): number {
  const factor = 10 ** places
  return Math.round(value * factor) / factor
}
