import { describe, expect, it, vi } from 'vitest'
import { lexicalGate } from '../src/safety/gate.js'
import { createMarketTools } from '../src/agent/tools/market.js'
import type { ChartRenderer } from '../src/chart/renderer.js'
import type { TokenMarket } from '../src/market/market.js'
import type { Candle } from '../src/market/types.js'
import type { XTools } from '../src/types.js'

const ADDRESS = '2AF7CqwieUjUPALL7icuZtL3X7wENdjUjGBMmfV2pump'

function candles(): Candle[] {
  return Array.from({ length: 60 }, (_, index) => ({
    time: 1_700_000_000 + index * 3_600,
    open: 0.0007 + index * 0.000_001,
    high: 0.00071 + index * 0.000_001,
    low: 0.00069 + index * 0.000_001,
    close: 0.000_705 + index * 0.000_001,
    volume: 100,
  }))
}

function market(): TokenMarket {
  return {
    overview: vi.fn().mockResolvedValue({
      address: ADDRESS,
      chain: 'solana',
      name: 'Tiny Humans AI',
      symbol: 'TINY',
      priceUsd: 0.000_747,
      marketCapUsd: 730_709,
      liquidityUsd: 93_428,
      volume24hUsd: 10_134,
      priceChange: { h24: 17.38 },
      txns24h: { buys: 79, sells: 75 },
      poolAddress: 'GEF9',
      source: 'dexscreener',
    }),
    candles: vi.fn().mockResolvedValue(candles()),
    flow: vi.fn().mockResolvedValue({
      window: { from: 'a', to: 'b', trades: 24 },
      buyVolumeUsd: 6_344,
      sellVolumeUsd: 2_687,
      netUsd: 3_657,
      uniqueWallets: 11,
      buyers: 7,
      sellers: 5,
      topBuyers: [{ wallet: 'WalletAAAAAAAAAA', boughtUsd: 1_600, soldUsd: 0, netUsd: 1_600, buys: 2, sells: 0, firstSeen: 'a', lastSeen: 'b' }],
      topSellers: [],
      known: [],
    }),
    holders: vi.fn(),
    wallet: vi.fn().mockResolvedValue(undefined),
    rankedTraders: vi.fn().mockResolvedValue([]),
  } as unknown as TokenMarket
}

function xTools(): XTools {
  return {
    searchPosts: vi.fn(),
    getPost: vi.fn(),
    getConversation: vi.fn(),
    createPost: vi.fn().mockResolvedValue({ id: '999' }),
    uploadMedia: vi.fn().mockResolvedValue({ mediaId: 'media-1' }),
  }
}

const renderer: ChartRenderer = {
  name: 'test',
  render: vi.fn().mockResolvedValue({
    svg: '<svg/>',
    png: Buffer.from('png-bytes'),
    width: 1_200,
    height: 675,
    path: '/tmp/chart.png',
  }),
}

function build(overrides: Partial<Parameters<typeof createMarketTools>[0]> = {}) {
  const x = overrides.x ?? xTools()
  const tools = createMarketTools({
    market: market(),
    address: ADDRESS,
    symbol: 'TINY',
    x,
    charts: renderer,
    dryRun: false,
    ...overrides,
  })
  return { x, tools: new Map(tools.map((tool) => [tool.name, tool])) }
}

describe('market tools', () => {
  it('publishes nothing that could move money, whatever the run', () => {
    const { tools } = build()
    for (const name of tools.keys()) {
      expect(name).not.toMatch(/airdrop|wallet|send|pay|transfer|swap|approve|sign/i)
    }
  })

  it('is the exact tool list, so a new capability is a conscious change', () => {
    const { tools } = build()
    expect([...tools.keys()]).toEqual([
      'token_overview',
      'market_context',
      'token_candles',
      'technical_analysis',
      'chart_timeframes',
      'token_progress',
      'render_chart',
      'token_holders',
      'token_flow',
      'trader_activity',
      'ranked_traders',
      'post_chart',
    ])
  })

  it('reports the source alongside the numbers, so nothing is quotable without one', async () => {
    const { tools } = build()
    const output = (await tools.get('token_overview')!.invoke({} as never)) as string
    expect(output).toContain('$730.7K')
    expect(output).toContain('source: dexscreener')
  })

  it('hands back a chart id rather than image bytes', async () => {
    const { tools } = build()
    const output = (await tools.get('render_chart')!.invoke({
      timeframe: '1h',
      bars: 60,
    } as never)) as string
    expect(output).toContain('chart_1')
    expect(output).toContain('1200x675')
  })

  it('attaches a rendered chart to the post it publishes', async () => {
    const { tools, x } = build()
    await tools.get('render_chart')!.invoke({ timeframe: '1h', bars: 60 } as never)
    const output = (await tools.get('post_chart')!.invoke({
      text: 'the level held',
      chartId: 'chart_1',
    } as never)) as string

    expect(x.uploadMedia).toHaveBeenCalledWith(
      expect.objectContaining({ mimeType: 'image/png' }),
    )
    expect(x.createPost).toHaveBeenCalledWith('the level held', { mediaIds: ['media-1'] })
    expect(output).toContain('with the chart attached')
  })

  it('refuses an id it never rendered instead of posting a bare text', async () => {
    const { tools, x } = build()
    const output = (await tools.get('post_chart')!.invoke({
      text: 'hello',
      chartId: 'chart_9',
    } as never)) as string
    expect(output).toMatch(/Refused/)
    expect(x.createPost).not.toHaveBeenCalled()
  })

  it('records the post it published so tomorrow can see the actual text', async () => {
    const onPublished = vi.fn().mockResolvedValue(undefined)
    const { tools } = build({ onPublished })
    await tools.get('render_chart')!.invoke({ timeframe: '1h', bars: 60 } as never)
    await tools.get('post_chart')!.invoke({ text: 'the level held', chartId: 'chart_1' } as never)

    expect(onPublished).toHaveBeenCalledWith({
      text: 'the level held',
      postedId: '999',
      chartId: 'chart_1',
      dryRun: false,
    })
  })

  it('records a rehearsal too, marked as one', async () => {
    const onPublished = vi.fn().mockResolvedValue(undefined)
    const { tools } = build({ dryRun: true, onPublished })
    await tools.get('post_chart')!.invoke({ text: 'draft' } as never)

    expect(onPublished).toHaveBeenCalledWith({ text: 'draft', dryRun: true })
  })

  /**
   * The post is already out by the time memory is written. Raising here would
   * read to the agent as a failed publish and invite it to post again.
   */
  it('does not report a memory outage as a failed post', async () => {
    const onPublished = vi.fn().mockRejectedValue(new Error('401 MISSING_TOKEN'))
    const { tools } = build({ onPublished })
    const output = (await tools.get('post_chart')!.invoke({ text: 'the level held' } as never)) as string

    expect(output).toContain('Published post 999')
  })

  it('publishes nothing in a dry run', async () => {
    const { tools, x } = build({ dryRun: true })
    const output = (await tools.get('post_chart')!.invoke({ text: 'hello' } as never)) as string
    expect(output).toContain('DRY RUN')
    expect(x.createPost).not.toHaveBeenCalled()
  })

  it('refuses outright when the agent is not allowed to publish', async () => {
    const { tools, x } = build({ allowPosting: false })
    const output = (await tools.get('post_chart')!.invoke({ text: 'hello' } as never)) as string
    expect(output).toMatch(/cannot publish/)
    expect(x.createPost).not.toHaveBeenCalled()
  })

  it('enforces the emoji cap the same way the reply path does', async () => {
    const { tools, x } = build({ maxEmoji: 1 })
    const output = (await tools.get('post_chart')!.invoke({
      text: 'up only 🚀🚀🚀',
    } as never)) as string
    expect(output).toMatch(/Refused/)
    expect(x.createPost).not.toHaveBeenCalled()
  })

  it('tells the agent the flow window is short rather than letting it say "today"', async () => {
    const { tools } = build()
    const output = (await tools.get('token_flow')!.invoke({ minVolumeUsd: 100 } as never)) as string
    expect(output).toContain('in the last few hours')
    expect(output).toContain('never name a wallet address')
  })

  it('says plainly that there is no KOL data rather than returning an empty list', async () => {
    const { tools } = build()
    const output = (await tools.get('ranked_traders')!.invoke({} as never)) as string
    expect(output).toMatch(/No ranked-trader source is configured/)
    expect(output).toMatch(/Do not claim any/)
  })

  it('leads with outperformance when the market is red and the token is not', async () => {
    const global = {
      snapshot: vi.fn().mockResolvedValue({
        totalMarketCapUsd: 2_699_012_632_815,
        totalChange24hPct: -1.8,
        assets: [{ symbol: 'BTC', priceUsd: 78_327, change24hPct: -1.11 }],
        source: 'coingecko',
      }),
    }
    const { tools } = build({ global: global as never })
    const output = (await tools.get('market_context')!.invoke({} as never)) as string

    expect(output).toContain('TRUE POSITIVES YOU CAN LEAD WITH')
    expect(output).toContain('outperforming')
    expect(output).toContain('The whole market is red today')
  })

  it('refuses to manufacture a comparison the token did not win', async () => {
    const global = {
      snapshot: vi.fn().mockResolvedValue({
        totalChange24hPct: 40,
        assets: [{ symbol: 'BTC', priceUsd: 78_327, change24hPct: 30 }],
        source: 'coingecko',
      }),
    }
    const { tools } = build({ global: global as never })
    const output = (await tools.get('market_context')!.invoke({} as never)) as string
    expect(output).toContain('Do not force a comparison that is not there')
  })

  it('says plainly when there is no global source rather than guessing', async () => {
    const { tools } = build()
    const output = (await tools.get('market_context')!.invoke({} as never)) as string
    expect(output).toContain('No global market source is configured')
  })

  it('reads the chart at four zoom levels and says whether they agree', async () => {
    const { tools } = build()
    const output = (await tools.get('chart_timeframes')!.invoke({} as never)) as string

    expect(output).toContain('hourly')
    expect(output).toContain('4-hour')
    expect(output).toContain('daily')
    expect(output).toContain('weekly')
    expect(output).toMatch(/timeframes read bullish|agreement is itself the story/)
  })

  it('tells the agent where to look when no timeframe is bullish on price', async () => {
    const falling = Array.from({ length: 60 }, (_, index) => ({
      time: 1_700_000_000 + index * 3_600,
      open: 0.002 - index * 0.000_02,
      high: 0.002 - index * 0.000_02,
      low: 0.0019 - index * 0.000_02,
      close: 0.0019 - index * 0.000_02,
      volume: 100,
    }))
    const market = {
      overview: vi.fn().mockResolvedValue({ priceUsd: 1, priceChange: {}, source: 'x', address: ADDRESS, chain: 'solana' }),
      candles: vi.fn().mockResolvedValue(falling),
    } as unknown as TokenMarket
    const tools = new Map(
      createMarketTools({
        market,
        address: ADDRESS,
        x: xTools(),
        charts: renderer,
        dryRun: true,
      }).map((tool) => [tool.name, tool]),
    )

    const output = (await tools.get('chart_timeframes')!.invoke({} as never)) as string
    expect(output).toContain('The positive is on-chain today')
  })

  it('reads the long horizon off daily candles', async () => {
    const { tools } = build()
    const output = (await tools.get('token_progress')!.invoke({ days: 120 } as never)) as string
    expect(output).toContain('days closed green')
    expect(output).toContain('long horizon')
  })

  it('carries the bullish score with the warning attached to it', async () => {
    const { tools } = build()
    const output = (await tools.get('technical_analysis')!.invoke({
      timeframe: '1h',
      bars: 60,
    } as never)) as string
    expect(output).toContain('bullish score:')
    expect(output).toContain('Below 40 the short-term tape is weak')
    expect(output).toContain('TRUE POSITIVES YOU CAN LEAD WITH')
  })
})

describe('post_chart content filter', () => {
  it('refuses a caption the filter holds', async () => {
    const { tools, x } = build({ safety: lexicalGate() })
    const output = (await tools.get('post_chart')!.invoke({ text: 'the chart is fucking green' } as never)) as string
    expect(output).toContain('Refused')
    expect(x.createPost).not.toHaveBeenCalled()
  })

  it('refuses a caption linking off the allowlist', async () => {
    const { tools, x } = build({ safety: lexicalGate() })
    const output = (await tools.get('post_chart')!.invoke({ text: 'chart at https://evil.xyz/x' } as never)) as string
    expect(output).toContain('not on the allowlist')
    expect(x.createPost).not.toHaveBeenCalled()
  })
})
