import { afterEach, describe, expect, it, vi } from 'vitest'
import { DexscreenerClient } from '../src/market/dexscreener.js'
import { GeckoTerminalClient } from '../src/market/geckoterminal.js'
import { GmgnClient } from '../src/market/gmgn.js'
import { summariseFlow } from '../src/market/market.js'
import type { Trade } from '../src/market/types.js'

function answering(body: unknown, ok = true): void {
  vi.stubGlobal(
    'fetch',
    vi.fn().mockResolvedValue({
      ok,
      status: ok ? 200 : 500,
      json: async () => body,
      text: async () => JSON.stringify(body),
    }),
  )
}

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('GeckoTerminal', () => {
  it('returns candles oldest first, whatever order they arrived in', async () => {
    answering({
      data: {
        attributes: {
          ohlcv_list: [
            [1_700_007_200, 3, 4, 2, 3.5, 10],
            [1_700_003_600, 2, 3, 1, 2.5, 20],
            [1_700_000_000, 1, 2, 0.5, 1.5, 30],
          ],
        },
      },
    })
    const candles = await new GeckoTerminalClient().candles({ pool: 'pool', timeframe: '1h' })
    expect(candles.map((candle) => candle.time)).toEqual([
      1_700_000_000, 1_700_003_600, 1_700_007_200,
    ])
  })

  it('drops a malformed row rather than charting a zero', async () => {
    answering({
      data: {
        attributes: {
          ohlcv_list: [
            [1_700_000_000, 1, 2, 0.5, 1.5, 30],
            [1_700_003_600, null, 3, 1, 2.5, 20],
          ],
        },
      },
    })
    const candles = await new GeckoTerminalClient().candles({ pool: 'pool', timeframe: '1h' })
    expect(candles).toHaveLength(1)
  })

  it('refuses to invent a price when the token is unknown', async () => {
    answering({ data: { attributes: {} } })
    await expect(new GeckoTerminalClient().overview('missing')).rejects.toThrow(/no price/)
  })

  it('reads the wallet and the side off each trade', async () => {
    answering({
      data: [
        {
          attributes: {
            tx_hash: '0xabc',
            tx_from_address: 'WalletOne',
            kind: 'buy',
            volume_in_usd: '104.47',
            block_timestamp: '2026-09-08T17:51:03Z',
            to_token_amount: '138670.49',
            price_to_in_usd: '0.00075',
          },
        },
      ],
    })
    const trades = await new GeckoTerminalClient().trades({ pool: 'pool', minVolumeUsd: 100 })
    expect(trades[0]).toMatchObject({ wallet: 'WalletOne', kind: 'buy', volumeUsd: 104.47 })
  })
})

describe('Dexscreener', () => {
  /**
   * The pool ranking bug, caught on live data: $TINY's deepest Solana pool held
   * $139K and did $336 of volume in a day, while the pool everybody actually
   * trades held $93K and did $10K. Ranking on liquidity charted the dead one.
   */
  it('picks the pool with the volume, not the one with the liquidity', async () => {
    answering({
      pairs: [
        {
          chainId: 'solana',
          baseToken: { address: 'TOKEN', symbol: 'TINY' },
          priceUsd: '0.0007',
          liquidity: { usd: 139_000 },
          volume: { h24: 336 },
          pairAddress: 'DeadPool',
        },
        {
          chainId: 'solana',
          baseToken: { address: 'TOKEN', symbol: 'TINY' },
          priceUsd: '0.00075',
          liquidity: { usd: 93_000 },
          volume: { h24: 10_134 },
          pairAddress: 'LivePool',
        },
      ],
    })
    const overview = await new DexscreenerClient().overview('TOKEN')
    expect(overview.poolAddress).toBe('LivePool')
    expect(overview.volume24hUsd).toBe(10_134)
  })

  it('ignores pairs where the token is the quote side', async () => {
    answering({
      pairs: [{ baseToken: { address: 'SOMETHING_ELSE' }, priceUsd: '1', volume: { h24: 99 } }],
    })
    await expect(new DexscreenerClient().overview('TOKEN')).rejects.toThrow(/no pair/)
  })
})

describe('GMGN', () => {
  it('surfaces a refusal instead of returning an empty chart', async () => {
    answering({ code: 1, msg: 'blocked' })
    await expect(
      new GmgnClient({ baseUrl: 'https://proxy.example' }).overview('TOKEN'),
    ).rejects.toThrow(/blocked/)
  })

  it('converts millisecond timestamps, which would otherwise chart 50,000 years out', async () => {
    answering({
      code: 0,
      data: { list: [{ time: 1_700_000_000_000, open: 1, high: 2, low: 0.5, close: 1.5, volume: 3 }] },
    })
    const candles = await new GmgnClient({ baseUrl: 'https://proxy.example' }).candles({
      address: 'TOKEN',
      timeframe: '1h',
    })
    expect(candles[0]!.time).toBe(1_700_000_000)
  })
})

describe('flow', () => {
  const trades: Trade[] = [
    { txHash: '1', timestamp: '2026-09-08T10:00:00Z', kind: 'buy', wallet: 'A', volumeUsd: 500, tokenAmount: 1, priceUsd: 1 },
    { txHash: '2', timestamp: '2026-09-08T11:00:00Z', kind: 'buy', wallet: 'A', volumeUsd: 300, tokenAmount: 1, priceUsd: 1 },
    { txHash: '3', timestamp: '2026-09-08T12:00:00Z', kind: 'sell', wallet: 'B', volumeUsd: 200, tokenAmount: 1, priceUsd: 1 },
  ]

  it('nets buys against sells and counts each wallet once', () => {
    const summary = summariseFlow(trades, new Map())
    expect(summary.netUsd).toBe(600)
    expect(summary.uniqueWallets).toBe(2)
    expect(summary.buyers).toBe(1)
    expect(summary.topBuyers[0]).toMatchObject({ wallet: 'A', boughtUsd: 800 })
  })

  it('reports the window it actually read, so a post cannot claim the whole day', () => {
    const summary = summariseFlow(trades, new Map())
    expect(summary.window).toMatchObject({
      from: '2026-09-08T10:00:00Z',
      to: '2026-09-08T12:00:00Z',
      trades: 3,
    })
  })

  it('labels a watchlist wallet, matching case-insensitively', () => {
    const summary = summariseFlow(trades, new Map([['a', 'a friend of the project']]))
    expect(summary.known.map((entry) => entry.label)).toEqual(['a friend of the project'])
  })

  it('says nothing rather than dividing by zero on an empty window', () => {
    const summary = summariseFlow([], new Map())
    expect(summary).toMatchObject({ netUsd: 0, uniqueWallets: 0 })
    expect(summary.window.trades).toBe(0)
  })
})

describe('the market layer', () => {
  it('builds a weekly series from daily candles, because the provider has none', async () => {
    const { TokenMarket } = await import('../src/market/market.js')
    const market = new TokenMarket()
    const daily = Array.from({ length: 70 }, (_, index) => ({
      time: 1_700_000_000 + index * 86_400,
      open: 1 + index,
      high: 2 + index,
      low: index,
      close: 1.5 + index,
      volume: 10,
    }))
    // Stand in for the network: the fold is what is under test, not the fetch.
    const fetched = vi.fn().mockResolvedValue(daily)
    Object.assign(market as unknown as Record<string, unknown>, { fetchCandles: fetched })

    const weekly = await market.candles({
      address: 'TOKEN',
      timeframe: '1w',
      limit: 5,
    })

    expect(fetched).toHaveBeenCalledWith('TOKEN', '1d', expect.any(Number))
    expect(weekly).toHaveLength(5)
    expect(weekly.every((candle, index) => index === 0 || candle.time > (weekly[index - 1]?.time ?? 0))).toBe(true)
  })

  it('serves the second read of a timeframe from cache, which is what keeps it under the rate limit', async () => {
    const { TokenMarket } = await import('../src/market/market.js')
    const market = new TokenMarket()
    const fetched = vi.fn().mockResolvedValue([])
    Object.assign(market as unknown as Record<string, unknown>, { fetchCandles: fetched })

    await market.candles({ address: 'TOKEN', timeframe: '1h', limit: 120 })
    await market.candles({ address: 'TOKEN', timeframe: '1h', limit: 120 })

    expect(fetched).toHaveBeenCalledTimes(1)
  })
})

describe('rate limits', () => {
  it('waits as long as the server asked rather than on its own schedule', async () => {
    const { fetchJson } = await import('../src/market/http.js')
    const slept: number[] = []
    let calls = 0
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => {
        calls += 1
        if (calls === 1) {
          return {
            ok: false,
            status: 429,
            headers: { get: (name: string) => (name === 'retry-after' ? '7' : null) },
            text: async () => 'slow down',
            json: async () => ({}),
          }
        }
        return { ok: true, status: 200, headers: { get: () => null }, json: async () => ({ fine: true }) }
      }),
    )

    const result = await fetchJson<{ fine: boolean }>('https://example.com/x', {
      sleep: async (ms) => {
        slept.push(ms)
      },
    })

    expect(result).toEqual({ fine: true })
    expect(slept).toEqual([7_000])
  })

  it('does not retry a 404, which would only spend the budget a 429 needs', async () => {
    const { fetchJson } = await import('../src/market/http.js')
    const fetcher = vi.fn(async () => ({
      ok: false,
      status: 404,
      headers: { get: () => null },
      text: async () => 'nope',
      json: async () => ({}),
    }))
    vi.stubGlobal('fetch', fetcher)

    await expect(fetchJson('https://example.com/x', { sleep: async () => undefined })).rejects.toThrow(
      /404/,
    )
    expect(fetcher).toHaveBeenCalledTimes(1)
  })
})
