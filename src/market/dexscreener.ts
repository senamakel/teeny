import { fetchJson, num } from './http.js'
import type { TokenOverview } from './types.js'

const BASE = 'https://api.dexscreener.com/latest/dex'

interface PairsResponse {
  pairs?: Array<Record<string, unknown>> | null
}

/**
 * Dexscreener, used for the snapshot only.
 *
 * It has the better overview -- five-minute and one-hour price change, buy and
 * sell counts, liquidity split by side -- and no OHLCV endpoint at all, so it
 * is paired with GeckoTerminal rather than replacing it.
 */
export class DexscreenerClient {
  constructor(private readonly options: { baseUrl?: string; timeoutMs?: number } = {}) {}

  async overview(address: string): Promise<TokenOverview> {
    const response = await fetchJson<PairsResponse>(
      `${this.options.baseUrl ?? BASE}/tokens/${address}`,
      this.options.timeoutMs === undefined ? {} : { timeoutMs: this.options.timeoutMs },
    )

    // A token trades in several pools and the quiet ones print prices nobody
    // could get filled at. Ranked by 24-hour volume rather than by liquidity,
    // because they disagree: $TINY's deepest Solana pool holds $139K and did
    // $336 of volume in a day, while the pool everyone actually trades holds
    // $93K and did $10K. Charting the first one draws a flat line through a
    // pool nothing happens in.
    const pairs = (response.pairs ?? []).filter(
      (pair) => (pair['baseToken'] as { address?: string } | undefined)?.address === address,
    )
    const pair = pairs.sort(
      (a, b) => volume24h(b) - volume24h(a) || liquidity(b) - liquidity(a),
    )[0]
    if (!pair) throw new Error(`Dexscreener knows no pair for ${address}`)

    const price = num(pair['priceUsd'])
    if (price === undefined) throw new Error(`Dexscreener returned no price for ${address}`)
    const base = (pair['baseToken'] ?? {}) as Record<string, unknown>
    const change = (pair['priceChange'] ?? {}) as Record<string, unknown>
    const volume = (pair['volume'] ?? {}) as Record<string, unknown>
    const day = ((pair['txns'] ?? {}) as Record<string, unknown>)['h24'] as
      | Record<string, unknown>
      | undefined
    const created = num(pair['pairCreatedAt'])

    return {
      address,
      chain: typeof pair['chainId'] === 'string' ? pair['chainId'] : 'unknown',
      ...(typeof base['name'] === 'string' ? { name: base['name'] } : {}),
      ...(typeof base['symbol'] === 'string' ? { symbol: base['symbol'] } : {}),
      priceUsd: price,
      ...maybe('marketCapUsd', num(pair['marketCap'])),
      ...maybe('fdvUsd', num(pair['fdv'])),
      ...maybe('liquidityUsd', num((pair['liquidity'] as Record<string, unknown>)?.['usd'])),
      ...maybe('volume24hUsd', num(volume['h24'])),
      priceChange: {
        ...maybe('m5', num(change['m5'])),
        ...maybe('h1', num(change['h1'])),
        ...maybe('h6', num(change['h6'])),
        ...maybe('h24', num(change['h24'])),
      },
      ...(num(day?.['buys']) !== undefined && num(day?.['sells']) !== undefined
        ? {
            txns24h: {
              buys: num(day?.['buys']) as number,
              sells: num(day?.['sells']) as number,
            },
          }
        : {}),
      ...(typeof pair['pairAddress'] === 'string' ? { poolAddress: pair['pairAddress'] } : {}),
      ...(typeof pair['dexId'] === 'string' ? { poolName: pair['dexId'] } : {}),
      ...(created === undefined ? {} : { pairCreatedAt: new Date(created).toISOString() }),
      source: 'dexscreener',
    }
  }
}

function liquidity(pair: Record<string, unknown>): number {
  return num((pair['liquidity'] as Record<string, unknown> | undefined)?.['usd']) ?? 0
}

function volume24h(pair: Record<string, unknown>): number {
  return num((pair['volume'] as Record<string, unknown> | undefined)?.['h24']) ?? 0
}

function maybe<K extends string, V>(key: K, value: V | undefined): Record<K, V> | Record<string, never> {
  return value === undefined ? {} : ({ [key]: value } as Record<K, V>)
}
