import { fetchJson, num } from '../market/http.js'

/** One row of a CoinGecko markets listing: what the moon chart needs and nothing else. */
export interface ListedCoin {
  id: string
  symbol: string
  name: string
  priceUsd: number
  marketCapUsd: number
  circulatingSupply: number
  /** CoinGecko's overall market cap rank. */
  rank?: number
  /** Position on the category board, 1-based, set when the coin came from one. */
  boardRank?: number
  change24hPct?: number
  athUsd?: number
  /** The coin's logo, as CoinGecko serves it. */
  imageUrl?: string
}

export interface LeaderboardSource {
  /** The category's coins, largest market cap first. */
  category(id: string, limit: number): Promise<ListedCoin[]>
  /** One coin by CoinGecko id. */
  coin(id: string): Promise<ListedCoin>
  /** The bytes of a logo, for embedding in the picture. Undefined when it cannot be had. */
  logo(url: string): Promise<{ data: Uint8Array; mimeType: string } | undefined>
}

const BASE = 'https://api.coingecko.com/api/v3'
/** CoinGecko's largest page. */
const PAGE = 250

/**
 * The AI meme coin leaderboard, from CoinGecko.
 *
 * The category listing (`/coins/markets?category=ai-meme-coins`) is the
 * same table as coingecko.com/en/categories/ai-meme-coins, sorted by market
 * cap, and it answers without a key. So does the row for the account's own
 * token, which is looked up by id rather than found in the list: at half a
 * million of market cap it is not on the first page of the category, and
 * the whole point of the post is where it would be.
 */
export class CoinGeckoLeaderboard implements LeaderboardSource {
  constructor(private readonly options: { baseUrl?: string; timeoutMs?: number; fetch?: typeof fetch } = {}) {}

  /**
   * The whole category, to `limit`, largest first. CoinGecko pages at 250
   * and the AI meme board is longer than that, so this walks the pages
   * until it has enough or a short page says there are no more. The board
   * position is the row's place in the merged list, which is what the
   * picture shows -- being #44 on the AI meme board means more than being
   * #3541 overall.
   */
  async category(id: string, limit: number): Promise<ListedCoin[]> {
    const coins: ListedCoin[] = []
    for (let page = 1; coins.length < limit; page += 1) {
      const rows = await this.markets(
        `category=${encodeURIComponent(id)}&order=market_cap_desc&per_page=${PAGE}&page=${page}`,
      )
      for (const row of rows) {
        const coin = parseCoin(row)
        if (coin) coins.push({ ...coin, boardRank: coins.length + 1 })
      }
      if (rows.length < PAGE) break
    }
    return coins.slice(0, limit)
  }

  async coin(id: string): Promise<ListedCoin> {
    const rows = await this.markets(`ids=${encodeURIComponent(id)}`)
    const coin = rows[0] ? parseCoin(rows[0]) : undefined
    if (!coin) throw new Error(`CoinGecko has no market row for "${id}"`)
    return coin
  }

  /**
   * A logo is a nicety: the picture reads without it, so nothing here
   * throws. Only PNG and JPEG come back, because those are what the
   * rasteriser can embed; a WebP or an SVG logo is treated as no logo.
   */
  async logo(url: string): Promise<{ data: Uint8Array; mimeType: string } | undefined> {
    const doFetch = this.options.fetch ?? fetch
    const controller = new AbortController()
    const timer = setTimeout(() => controller.abort(), this.options.timeoutMs ?? 10_000)
    try {
      const parsed = new URL(url)
      if (parsed.protocol !== 'https:') return undefined
      const response = await doFetch(url, { signal: controller.signal })
      if (!response.ok) return undefined
      const data = new Uint8Array(await response.arrayBuffer())
      const mimeType = sniffImage(data)
      return mimeType ? { data, mimeType } : undefined
    } catch {
      return undefined
    } finally {
      clearTimeout(timer)
    }
  }

  private markets(query: string): Promise<Array<Record<string, unknown>>> {
    const base = this.options.baseUrl ?? BASE
    return fetchJson<Array<Record<string, unknown>>>(
      `${base}/coins/markets?vs_currency=usd&${query}`,
      this.options.timeoutMs === undefined ? {} : { timeoutMs: this.options.timeoutMs },
    )
  }
}

/** A row with no price, cap or supply cannot be placed on the ladder and is dropped. */
export function parseCoin(row: Record<string, unknown>): ListedCoin | undefined {
  const id = typeof row['id'] === 'string' ? row['id'] : undefined
  const symbol = typeof row['symbol'] === 'string' ? row['symbol'].toUpperCase() : undefined
  const name = typeof row['name'] === 'string' ? row['name'] : symbol
  const priceUsd = num(row['current_price'])
  const marketCapUsd = num(row['market_cap'])
  const circulatingSupply = num(row['circulating_supply'])
  if (!id || !symbol || !name || priceUsd === undefined || marketCapUsd === undefined) return undefined
  if (circulatingSupply === undefined || circulatingSupply <= 0 || marketCapUsd <= 0) return undefined
  const rank = num(row['market_cap_rank'])
  const change = num(row['price_change_percentage_24h'])
  const ath = num(row['ath'])
  return {
    id,
    symbol,
    name,
    priceUsd,
    marketCapUsd,
    circulatingSupply,
    ...(rank === undefined ? {} : { rank }),
    ...(change === undefined ? {} : { change24hPct: change }),
    ...(ath === undefined ? {} : { athUsd: ath }),
    ...(typeof row['image'] === 'string' && row['image'] ? { imageUrl: row['image'] } : {}),
  }
}

/** PNG or JPEG by their magic bytes; anything else is not embeddable. */
export function sniffImage(data: Uint8Array): 'image/png' | 'image/jpeg' | undefined {
  if (data.length > 8 && data[0] === 0x89 && data[1] === 0x50 && data[2] === 0x4e && data[3] === 0x47) return 'image/png'
  if (data.length > 3 && data[0] === 0xff && data[1] === 0xd8 && data[2] === 0xff) return 'image/jpeg'
  return undefined
}
