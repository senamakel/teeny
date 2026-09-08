import { fetchJson, num } from './http.js'
import type { Candle, Timeframe, TokenOverview } from './types.js'
import { TIMEFRAMES } from './types.js'

/** A wallet GMGN has ranked on this token. */
export interface RankedTrader {
  wallet: string
  realizedProfitUsd?: number
  boughtUsd?: number
  soldUsd?: number
  holdingAmount?: number
  tags: string[]
}

interface Envelope<T> {
  code?: number
  msg?: string
  data?: T
}

/**
 * GMGN.
 *
 * GMGN has the data this account would most like -- smart-money labels, KOL
 * wallets, top-trader PnL -- and no public API. `gmgn.ai` fronts everything
 * with Cloudflare's bot check, which answers a server with an HTML
 * interstitial and HTTP 403 no matter what headers it sends, on both the
 * token-quotation and the candles endpoints.
 *
 * So this client exists but is off unless `GMGN_BASE_URL` names something that
 * will actually answer -- a proxy, a scraping service, a cached mirror. When it
 * is unset the market layer runs on GeckoTerminal and Dexscreener and the KOL
 * tools fall back to the configured watchlist. Nothing silently invents data:
 * every method here throws if GMGN answers with anything but JSON.
 */
export class GmgnClient {
  constructor(
    private readonly options: {
      baseUrl: string
      chain?: string
      timeoutMs?: number
      headers?: Record<string, string>
    },
  ) {}

  private get chain(): string {
    return this.options.chain ?? 'sol'
  }

  private get<T>(path: string): Promise<Envelope<T>> {
    return fetchJson<Envelope<T>>(`${this.options.baseUrl.replace(/\/$/, '')}${path}`, {
      ...(this.options.timeoutMs === undefined ? {} : { timeoutMs: this.options.timeoutMs }),
      ...(this.options.headers ? { headers: this.options.headers } : {}),
    })
  }

  private static unwrap<T>(envelope: Envelope<T>, what: string): T {
    if (envelope.code !== undefined && envelope.code !== 0) {
      throw new Error(`GMGN refused ${what}: ${envelope.msg ?? `code ${envelope.code}`}`)
    }
    if (!envelope.data) throw new Error(`GMGN returned no data for ${what}`)
    return envelope.data
  }

  async overview(address: string): Promise<TokenOverview> {
    const data = GmgnClient.unwrap(
      await this.get<{ token?: Record<string, unknown> }>(
        `/defi/quotation/v1/tokens/${this.chain}/${address}`,
      ),
      'token overview',
    )
    const token = data.token ?? (data as unknown as Record<string, unknown>)
    const price = num(token['price'])
    if (price === undefined) throw new Error(`GMGN returned no price for ${address}`)

    return {
      address,
      chain: this.chain,
      ...(typeof token['name'] === 'string' ? { name: token['name'] } : {}),
      ...(typeof token['symbol'] === 'string' ? { symbol: token['symbol'] } : {}),
      priceUsd: price,
      ...only('marketCapUsd', num(token['market_cap'])),
      ...only('fdvUsd', num(token['fdv'])),
      ...only('liquidityUsd', num(token['liquidity'])),
      ...only('volume24hUsd', num(token['volume_24h'])),
      priceChange: {
        ...only('m5', num(token['price_change_percent5m'])),
        ...only('h1', num(token['price_change_percent1h'])),
        ...only('h6', num(token['price_change_percent6h'])),
        ...only('h24', num(token['price_change_percent24h'])),
      },
      ...(num(token['buys_24h']) !== undefined && num(token['sells_24h']) !== undefined
        ? {
            txns24h: {
              buys: num(token['buys_24h']) as number,
              sells: num(token['sells_24h']) as number,
            },
          }
        : {}),
      ...(typeof token['pool_address'] === 'string' ? { poolAddress: token['pool_address'] } : {}),
      source: 'gmgn',
    }
  }

  async candles(input: {
    address: string
    timeframe: Timeframe
    limit?: number
  }): Promise<Candle[]> {
    const spec = TIMEFRAMES[input.timeframe]
    const limit = Math.min(Math.max(input.limit ?? 100, 1), 1_000)
    const to = Math.floor(Date.now() / 1_000)
    const from = to - spec.seconds * limit
    const data = GmgnClient.unwrap(
      await this.get<{ list?: Array<Record<string, unknown>> }>(
        `/api/v1/token_candles/${this.chain}/${input.address}` +
          `?resolution=${input.timeframe}&from=${from}&to=${to}`,
      ),
      'candles',
    )

    return (data.list ?? [])
      .flatMap((row) => {
        const time = num(row['time']) ?? num(row['timestamp'])
        const open = num(row['open'])
        const high = num(row['high'])
        const low = num(row['low'])
        const close = num(row['close'])
        if ([time, open, high, low, close].some((value) => value === undefined)) return []
        return [
          {
            // GMGN answers in milliseconds where GeckoTerminal answers in
            // seconds, and a chart drawn on the wrong one is 50,000 years out.
            time: (time as number) > 1e11 ? Math.floor((time as number) / 1_000) : (time as number),
            open: open as number,
            high: high as number,
            low: low as number,
            close: close as number,
            volume: num(row['volume']) ?? 0,
          },
        ]
      })
      .sort((a, b) => a.time - b.time)
  }

  /** The wallets GMGN ranks on this token, with whatever labels it carries. */
  async topTraders(address: string): Promise<RankedTrader[]> {
    const data = GmgnClient.unwrap(
      await this.get<Array<Record<string, unknown>> | { list?: Array<Record<string, unknown>> }>(
        `/defi/quotation/v1/tokens/top_traders/${this.chain}/${address}`,
      ),
      'top traders',
    )
    const rows = Array.isArray(data) ? data : (data.list ?? [])
    return rows.flatMap((row) => {
      const wallet = row['address'] ?? row['wallet_address']
      if (typeof wallet !== 'string') return []
      const tags = row['tags']
      return [
        {
          wallet,
          ...only('realizedProfitUsd', num(row['realized_profit'])),
          ...only('boughtUsd', num(row['buy_volume_cur'] ?? row['bought_usd'])),
          ...only('soldUsd', num(row['sell_volume_cur'] ?? row['sold_usd'])),
          ...only('holdingAmount', num(row['balance'] ?? row['holding_amount'])),
          tags: Array.isArray(tags) ? tags.filter((tag): tag is string => typeof tag === 'string') : [],
        },
      ]
    })
  }
}

function only<K extends string, V>(key: K, value: V | undefined): Record<K, V> | Record<string, never> {
  return value === undefined ? {} : ({ [key]: value } as Record<K, V>)
}
