import { afterEach, describe, expect, it, vi } from 'vitest'
import { GlobalMarketClient, relativeStrength, type GlobalSnapshot } from '../src/market/global.js'

function answering(bodies: Record<string, unknown>): void {
  vi.stubGlobal(
    'fetch',
    vi.fn(async (url: string) => {
      const key = Object.keys(bodies).find((part) => url.includes(part))
      return {
        ok: key !== undefined,
        status: key === undefined ? 404 : 200,
        json: async () => bodies[key ?? ''],
        text: async () => '',
      }
    }),
  )
}

afterEach(() => {
  vi.unstubAllGlobals()
})

const market: GlobalSnapshot = {
  totalMarketCapUsd: 2_699_012_632_815,
  totalChange24hPct: -1.8,
  assets: [
    { symbol: 'BTC', priceUsd: 78_327, change24hPct: -1.11 },
    { symbol: 'SOL', priceUsd: 103.33, change24hPct: -0.61 },
  ],
  source: 'coingecko',
}

describe('global market', () => {
  it('reads the majors and the whole-market change', async () => {
    answering({
      '/global': {
        data: {
          total_market_cap: { usd: 2_699_012_632_815 },
          market_cap_change_percentage_24h_usd: -1.8,
          market_cap_percentage: { btc: 58.3 },
        },
      },
      '/simple/price': {
        bitcoin: { usd: 78_327, usd_24h_change: -1.11 },
        solana: { usd: 103.33, usd_24h_change: -0.61 },
      },
    })

    const snapshot = await new GlobalMarketClient().snapshot()
    expect(snapshot.totalChange24hPct).toBe(-1.8)
    expect(snapshot.btcDominancePct).toBe(58.3)
    expect(snapshot.assets.map((asset) => asset.symbol)).toEqual(['BTC', 'SOL'])
  })

  it('still answers when one of the two endpoints is down', async () => {
    answering({
      '/simple/price': { bitcoin: { usd: 78_327, usd_24h_change: -1.11 } },
    })
    const snapshot = await new GlobalMarketClient().snapshot()
    expect(snapshot.assets).toHaveLength(1)
    expect(snapshot.totalMarketCapUsd).toBeUndefined()
  })
})

describe('relative strength', () => {
  /** The sentence the account exists to be able to say on a red day. */
  it('finds the outperformance when the market is down and the token is not', () => {
    const strength = relativeStrength(14.2, market)
    expect(strength.marketDirection).toBe('down')
    expect(strength.beating[0]).toContain('the whole crypto market')
    expect(strength.beating[0]).toContain('16 points')
  })

  it('ranks the biggest gap first, so the best true comparison is the easiest to reach', () => {
    const strength = relativeStrength(14.2, market)
    const points = strength.against
      .filter((entry) => entry.points > 0)
      .map((entry) => entry.points)
    expect([...points].sort((a, b) => b - a)[0]).toBe(
      Math.max(...points),
    )
    expect(strength.beating).toHaveLength(3)
  })

  it('offers nothing rather than a losing comparison', () => {
    const strength = relativeStrength(-9, market)
    expect(strength.beating).toEqual([])
  })

  it('calls a flat market flat rather than a direction', () => {
    expect(
      relativeStrength(2, { ...market, totalChange24hPct: 0.1 }).marketDirection,
    ).toBe('flat')
  })
})
