import { fetchJson, num } from './http.js'
import { TIMEFRAMES, type Candle, type MarketSource, type Timeframe, type TokenOverview, type Trade } from './types.js'

const BASE = 'https://api.geckoterminal.com/api/v2'

interface TokenResponse {
  data?: {
    attributes?: Record<string, unknown> & { volume_usd?: Record<string, unknown> }
    relationships?: { top_pools?: { data?: Array<{ id?: string }> } }
  }
  included?: Array<{ id?: string; attributes?: Record<string, unknown> }>
}

interface OhlcvResponse {
  data?: { attributes?: { ohlcv_list?: unknown[][] } }
}

interface TradesResponse {
  data?: Array<{ attributes?: Record<string, unknown> }>
}

/** `solana_ABC` -> `ABC`. Every GeckoTerminal id is namespaced by network. */
function bare(id: string | undefined): string | undefined {
  if (!id) return undefined
  const index = id.indexOf('_')
  return index >= 0 ? id.slice(index + 1) : id
}

/**
 * GeckoTerminal, the free public DEX data API.
 *
 * It is the default source rather than GMGN because it answers a server: GMGN
 * fronts its API with Cloudflare's bot check, which returns an interstitial to
 * anything without a browser fingerprint. `GmgnClient` exists for when there is
 * a proxy in front of it; this is what runs when there is not.
 */
export class GeckoTerminalClient implements MarketSource {
  constructor(
    private readonly options: { network?: string; baseUrl?: string; timeoutMs?: number } = {},
  ) {}

  private get network(): string {
    return this.options.network ?? 'solana'
  }

  private get(path: string): Promise<unknown> {
    return fetchJson(
      `${this.options.baseUrl ?? BASE}${path}`,
      this.options.timeoutMs === undefined ? {} : { timeoutMs: this.options.timeoutMs },
    )
  }

  async overview(address: string): Promise<TokenOverview> {
    const response = (await this.get(
      `/networks/${this.network}/tokens/${address}?include=top_pools`,
    )) as TokenResponse
    const attributes = response.data?.attributes ?? {}
    const price = num(attributes['price_usd'])
    if (price === undefined) {
      throw new Error(`GeckoTerminal returned no price for ${address}`)
    }

    // The first related pool is the deepest one, which is the pool a chart
    // should be drawn from: a thin pool's candles are mostly noise.
    const poolId = response.data?.relationships?.top_pools?.data?.[0]?.id
    const pool = response.included?.find((entry) => entry.id === poolId)?.attributes ?? {}
    const changes = (pool['price_percent_changes'] ?? {}) as Record<string, unknown>
    const transactions = (pool['transactions'] ?? {}) as Record<string, unknown>
    const day = (transactions['h24'] ?? {}) as Record<string, unknown>

    return {
      address,
      chain: this.network,
      ...(typeof attributes['name'] === 'string' ? { name: attributes['name'] } : {}),
      ...(typeof attributes['symbol'] === 'string' ? { symbol: attributes['symbol'] } : {}),
      priceUsd: price,
      ...pick('marketCapUsd', num(attributes['market_cap_usd'])),
      ...pick('fdvUsd', num(attributes['fdv_usd'])),
      ...pick('liquidityUsd', num(attributes['total_reserve_in_usd'])),
      ...pick(
        'volume24hUsd',
        num((attributes['volume_usd'] as Record<string, unknown> | undefined)?.['h24']),
      ),
      priceChange: {
        ...pick('h1', percent(changes['last_1h'])),
        ...pick('h6', percent(changes['last_6h'])),
        ...pick('h24', percent(changes['last_24h'])),
      },
      ...(num(day['buys']) !== undefined && num(day['sells']) !== undefined
        ? { txns24h: { buys: num(day['buys']) as number, sells: num(day['sells']) as number } }
        : {}),
      ...pick('poolAddress', bare(poolId)),
      ...(typeof pool['name'] === 'string' ? { poolName: pool['name'] } : {}),
      ...(typeof pool['pool_created_at'] === 'string'
        ? { pairCreatedAt: pool['pool_created_at'] }
        : {}),
      source: 'geckoterminal',
    }
  }

  /**
   * OHLCV for one pool, oldest first.
   *
   * GeckoTerminal answers newest first, which is the opposite of what every
   * indicator here expects; reversing it at the edge means nothing downstream
   * has to remember which way round a provider happens to be.
   */
  async candles(input: { pool: string; timeframe: Timeframe; limit?: number }): Promise<Candle[]> {
    const spec = TIMEFRAMES[input.timeframe]
    const limit = Math.min(Math.max(input.limit ?? 100, 1), 1_000)
    const response = (await this.get(
      `/networks/${this.network}/pools/${input.pool}/ohlcv/${spec.unit}` +
        `?aggregate=${spec.aggregate}&limit=${limit}&currency=usd`,
    )) as OhlcvResponse

    const rows = response.data?.attributes?.ohlcv_list ?? []
    const candles = rows.flatMap((row) => {
      const [time, open, high, low, close, volume] = row.map((value) => num(value))
      if (
        time === undefined ||
        open === undefined ||
        high === undefined ||
        low === undefined ||
        close === undefined
      ) {
        return []
      }
      return [{ time, open, high, low, close, volume: volume ?? 0 }]
    })
    return candles.sort((a, b) => a.time - b.time)
  }

  /** Recent swaps over a size floor, newest first, with the wallet that made each. */
  async trades(input: { pool: string; minVolumeUsd?: number }): Promise<Trade[]> {
    const floor = Math.max(input.minVolumeUsd ?? 0, 0)
    const response = (await this.get(
      `/networks/${this.network}/pools/${input.pool}/trades` +
        `?trade_volume_in_usd_greater_than=${floor}`,
    )) as TradesResponse

    return (response.data ?? []).flatMap((entry) => {
      const attributes = entry.attributes ?? {}
      const wallet = attributes['tx_from_address']
      const hash = attributes['tx_hash']
      const volume = num(attributes['volume_in_usd'])
      if (typeof wallet !== 'string' || typeof hash !== 'string' || volume === undefined) return []
      const buy = attributes['kind'] === 'buy'
      return [
        {
          txHash: hash,
          timestamp:
            typeof attributes['block_timestamp'] === 'string'
              ? attributes['block_timestamp']
              : new Date().toISOString(),
          kind: buy ? ('buy' as const) : ('sell' as const),
          wallet,
          volumeUsd: volume,
          tokenAmount:
            num(attributes[buy ? 'to_token_amount' : 'from_token_amount']) ?? 0,
          priceUsd: num(attributes[buy ? 'price_to_in_usd' : 'price_from_in_usd']) ?? 0,
        },
      ]
    })
  }
}

/** Adds a key only when there is a value, which `exactOptionalPropertyTypes` requires. */
function pick<K extends string, V>(key: K, value: V | undefined): Record<K, V> | Record<string, never> {
  return value === undefined ? {} : ({ [key]: value } as Record<K, V>)
}

/** GeckoTerminal spells a percentage "+12.4%"; everything here wants 12.4. */
function percent(value: unknown): number | undefined {
  if (typeof value === 'string') return num(value.replace(/[%+]/g, ''))
  return num(value)
}
