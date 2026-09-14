import { describe, expect, it, vi } from 'vitest'
import { MoonJob } from '../src/jobs/moon-job.js'
import type { Journal } from '../src/journal/journal.js'
import type { TextModel } from '../src/media/reference.js'
import { ladder, pickTarget, rung, usedSymbols } from '../src/moon/ladder.js'
import { CoinGeckoLeaderboard, parseCoin, sniffImage, type LeaderboardSource, type ListedCoin } from '../src/moon/leaderboard.js'
import { MoonPipeline, printable, type MoonPipelineOptions, type MoonResult } from '../src/moon/pipeline.js'
import { moonCaptionPrompt } from '../src/moon/prompt.js'
import { multiple, renderMoonSvg } from '../src/moon/svg.js'
import { lexicalGate } from '../src/safety/gate.js'
import type { Memory, XTools } from '../src/types.js'

const PNG = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0])

function coin(symbol: string, marketCapUsd: number, overrides: Partial<ListedCoin> = {}): ListedCoin {
  return {
    id: symbol.toLowerCase(),
    symbol,
    name: symbol,
    priceUsd: marketCapUsd / 1_000_000_000,
    marketCapUsd,
    circulatingSupply: 1_000_000_000,
    ...overrides,
  }
}

const tiny = coin('TINY', 500_000, { id: 'tiny-humans-ai', priceUsd: 0.0005, circulatingSupply: 1_000_000_000, change24hPct: 8.3, boardRank: 9 })
const board = [
  coin('TIBBIR', 248_000_000, { rank: 150, boardRank: 1 }),
  coin('FARTCOIN', 143_000_000),
  coin('TURBO', 63_000_000),
  coin('AIC', 32_000_000),
  coin('DRB', 18_500_000, { boardRank: 5 }),
  coin('CRABAI', 14_700_000),
  coin('LUNA', 4_900_000),
  coin('AIDOGE', 1_950_000),
  coin('SMALL', 600_000),
  tiny,
  coin('BELOW', 100_000),
]

describe('the arithmetic', () => {
  it('prices the token at the target cap with its own supply', () => {
    const result = rung(tiny, board[1]!)
    expect(result.multiple).toBeCloseTo(286, 0)
    expect(result.impliedPriceUsd).toBeCloseTo(0.143, 4)
  })

  it('formats a multiple the way a reader says it', () => {
    expect(multiple(490.07)).toBe('490x')
    expect(multiple(37.04)).toBe('37x')
    expect(multiple(64.8)).toBe('64.8x')
    expect(multiple(2.5)).toBe('2.5x')
    expect(multiple(Number.NaN)).toBe('—')
  })
})

describe('picking the yardstick', () => {
  it('only picks above the floor, inside the ceiling, and never itself', () => {
    const picks = new Set<string>()
    for (let step = 0; step < 50; step += 1) {
      const pick = pickTarget(tiny, [...board, tiny], { random: () => step / 50, maxMultiple: 100 })
      picks.add(pick!.symbol)
    }
    // 2x..100x of $500K: $1M..$50M.
    expect([...picks].sort()).toEqual(['AIC', 'AIDOGE', 'CRABAI', 'DRB', 'LUNA'])
  })

  it('skips what memory says was used, and resets when everything was', () => {
    const inBand = { minMultiple: 2, maxMultiple: 100, random: () => 0 }
    expect(pickTarget(tiny, board, inBand)!.symbol).toBe('AIC')
    expect(pickTarget(tiny, board, { ...inBand, exclude: new Set(['AIC']) })!.symbol).toBe('DRB')
    const all = new Set(['AIC', 'AIDOGE', 'CRABAI', 'DRB', 'LUNA'])
    expect(pickTarget(tiny, board, { ...inBand, exclude: all })!.symbol).toBe('AIC')
  })

  it('falls back to anything above when nothing is inside the ceiling, and to nothing when nothing is above', () => {
    expect(pickTarget(tiny, board, { minMultiple: 400, maxMultiple: 450, random: () => 0 })!.symbol).toBe('TIBBIR')
    expect(pickTarget(tiny, [coin('BELOW', 100_000)])).toBeUndefined()
  })

  it('reads used tickers out of recalled prose, whole words, with or without the cashtag', () => {
    const recalled = 'Posted $FARTCOIN at 286x on Monday; before that TURBO (127x). Someone mentioned the AIC token too.'
    expect([...usedSymbols(recalled, board)].sort()).toEqual(['AIC', 'FARTCOIN', 'TURBO'])
    // "AIDOGE" contains "AI" but the board has no AI; "LUNA" is not in "Lunar".
    expect(usedSymbols('Lunar eclipse tonight', board).size).toBe(0)
    expect(usedSymbols('', board).size).toBe(0)
  })
})

describe('the ladder', () => {
  it('spreads the board across the climb, keeps the target, and sorts ascending', () => {
    const target = board[4]! // DRB
    const rungs = ladder(tiny, board, target, 5)
    expect(rungs).toHaveLength(5)
    expect(rungs.map((entry) => entry.coin.symbol)).toContain('DRB')
    expect(rungs.map((entry) => entry.coin.symbol)).toContain('TIBBIR')
    expect(rungs.map((entry) => entry.coin.symbol)).not.toContain('BELOW')
    for (let index = 1; index < rungs.length; index += 1) {
      expect(rungs[index]!.coin.marketCapUsd).toBeGreaterThan(rungs[index - 1]!.coin.marketCapUsd)
    }
  })

  it('leaves neighbours under the floor off the strip', () => {
    const target = board[4]!
    expect(ladder(tiny, board, target, 12).map((entry) => entry.coin.symbol)).not.toContain('SMALL')
    expect(ladder(tiny, board, target, 12, 1).map((entry) => entry.coin.symbol)).toContain('SMALL')
  })

  it('draws the target alone when nothing else is above', () => {
    const target = coin('ONLY', 5_000_000)
    expect(ladder(tiny, [target, coin('BELOW', 100_000)], target, 7).map((entry) => entry.coin.symbol)).toEqual(['ONLY'])
  })
})

describe('the picture', () => {
  const input = {
    token: tiny,
    target: rung(tiny, board[4]!),
    ladder: ladder(tiny, board, board[4]!, 7),
    boardName: 'AI meme coins',
    boardSize: 12,
    date: '2026-09-14',
    watermark: '@TinyMascot',
  }

  it('says what it is, in the headline and the fine print', () => {
    const svg = renderMoonSvg(input)
    expect(svg).toContain('$TINY WITH THE MARKET CAP OF $DRB')
    expect(svg).toContain('>VS<')
    expect(svg).toContain('>$DRB<')
    expect(svg).toContain('$0.0185') // 18.5M / 1B
    expect(svg).toContain('>37x<')
    expect(svg).toContain('#9 of 12 AI meme coins')
    expect(svg).toContain('#5 of 12 AI meme coins')
    expect(svg).toContain('Arithmetic, not a forecast.')
    expect(svg).toContain('AI meme coins (12 coins) · CoinGecko · 2026-09-14')
    expect(svg).toContain('@TinyMascot')
  })

  it('lights the target rung and draws the others', () => {
    const svg = renderMoonSvg(input)
    const lit = svg.match(/fill="#7c6cff33"/g) ?? []
    expect(lit).toHaveLength(1)
    for (const entry of input.ladder) expect(svg).toContain(`>$${entry.coin.symbol}<`)
  })

  it('embeds both logos as data URIs and falls back to a lettered disc without one', () => {
    const withLogos = renderMoonSvg({ ...input, tokenLogo: { data: PNG, mimeType: 'image/png' }, targetLogo: { data: PNG, mimeType: 'image/jpeg' } })
    expect(withLogos.match(/href="data:image\/png;base64,/g)).toHaveLength(2) // the fighter and the strip
    expect(withLogos).toContain('href="data:image/jpeg;base64,')
    const without = renderMoonSvg(input)
    expect(without).not.toContain('data:image')
    expect(without).toContain('>D<') // the disc's initial for $DRB
    expect(without).toContain('>T<') // and for $TINY
  })

  it('keeps rung labels apart along the strip when coins are almost the same size', () => {
    const near = [coin('A', 10_000_000), coin('B', 10_100_000), coin('C', 10_200_000), coin('D', 10_300_000)]
    const svg = renderMoonSvg({ ...input, ladder: near.map((entry) => rung(tiny, entry)), target: rung(tiny, near[0]!) })
    const xs = [...svg.matchAll(/<circle cx="([\d.]+)" cy="562" r="(?:5|8)"/g)].map((match) => Number(match[1])).sort((a, b) => a - b)
    expect(xs).toHaveLength(4)
    // Alternate lanes, so within a lane the gap is the minimum.
    expect(xs[2]! - xs[0]!).toBeGreaterThanOrEqual(95.9)
    expect(xs[3]! - xs[1]!).toBeGreaterThanOrEqual(95.9)
  })

  it('escapes a coin name that carries markup', () => {
    const nasty = coin('X', 5_000_000, { name: '<script>&"' })
    const svg = renderMoonSvg({ ...input, target: rung(tiny, nasty), ladder: [rung(tiny, nasty)] })
    expect(svg).not.toContain('<script>')
    expect(svg).toContain('&lt;script&gt;&amp;&quot;')
  })
})

describe('the leaderboard client', () => {
  it('parses a CoinGecko row and drops one it cannot place', () => {
    const parsed = parseCoin({
      id: 'fartcoin',
      symbol: 'fartcoin',
      name: 'Fartcoin',
      current_price: 0.1428,
      market_cap: 142_859_822,
      circulating_supply: 999_972_851,
      market_cap_rank: 217,
      price_change_percentage_24h: -1.2,
      ath: 2.48,
      image: 'https://coin-images.coingecko.com/x.jpg',
    })
    expect(parsed).toMatchObject({ id: 'fartcoin', symbol: 'FARTCOIN', rank: 217, change24hPct: -1.2, athUsd: 2.48 })
    expect(parseCoin({ id: 'x', symbol: 'x', name: 'x', current_price: 1, market_cap: null, circulating_supply: 1 })).toBeUndefined()
    expect(parseCoin({ id: 'x', symbol: 'x', name: 'x', current_price: 1, market_cap: 1, circulating_supply: 0 })).toBeUndefined()
  })

  it('pages through the category by market cap, numbers the board, and fetches the coin by id', async () => {
    const calls: string[] = []
    const row = (index: number) => ({ id: `c${index}`, symbol: `c${index}`, name: `C${index}`, current_price: 1, market_cap: 1e9 - index, circulating_supply: 1e6 })
    const doFetch = vi.fn(async (url: string | URL | Request) => {
      calls.push(String(url))
      const text = String(url)
      const body = text.includes('ids=')
        ? [{ id: 'tiny-humans-ai', symbol: 'tiny', name: 'Tiny', current_price: 0.0005, market_cap: 500_000, circulating_supply: 1e9 }]
        : text.includes('page=1')
          ? Array.from({ length: 250 }, (_, index) => row(index + 1))
          : [row(251), row(252)]
      return new Response(JSON.stringify(body), { status: 200, headers: { 'content-type': 'application/json' } })
    })
    vi.stubGlobal('fetch', doFetch)
    try {
      const client = new CoinGeckoLeaderboard({ baseUrl: 'https://gecko.test/api/v3' })
      const [own, list] = await Promise.all([client.coin('tiny-humans-ai'), client.category('ai-meme-coins', 500)])
      expect(own.symbol).toBe('TINY')
      expect(list).toHaveLength(252)
      expect(list[0]!.boardRank).toBe(1)
      expect(list[251]!.boardRank).toBe(252)
      expect(calls.filter((url) => url.includes('category=ai-meme-coins&order=market_cap_desc&per_page=250'))).toHaveLength(2)
      expect(calls.some((url) => url.includes('ids=tiny-humans-ai'))).toBe(true)
      // A limit inside the first page stops there.
      calls.length = 0
      expect(await client.category('ai-meme-coins', 10)).toHaveLength(10)
      expect(calls).toHaveLength(1)
    } finally {
      vi.unstubAllGlobals()
    }
  })

  it('fetches a logo only over https, only as PNG or JPEG, and never throws', async () => {
    const client = new CoinGeckoLeaderboard({
      fetch: vi.fn(async () => new Response(PNG, { status: 200 })) as unknown as typeof fetch,
    })
    expect((await client.logo('https://coin-images.coingecko.com/a.png'))?.mimeType).toBe('image/png')
    expect(await client.logo('http://coin-images.coingecko.com/a.png')).toBeUndefined()
    const webp = new CoinGeckoLeaderboard({
      fetch: vi.fn(async () => new Response(new Uint8Array([0x52, 0x49, 0x46, 0x46, 0, 0, 0, 0, 0x57]), { status: 200 })) as unknown as typeof fetch,
    })
    expect(await webp.logo('https://x.test/a.webp')).toBeUndefined()
    const down = new CoinGeckoLeaderboard({ fetch: vi.fn(async () => { throw new Error('down') }) as unknown as typeof fetch })
    expect(await down.logo('https://x.test/a.png')).toBeUndefined()
    expect(sniffImage(new Uint8Array([0xff, 0xd8, 0xff, 0xe0]))).toBe('image/jpeg')
  })
})

describe('the caption prompt', () => {
  it('hands the model the arithmetic and the rules, and calls the yardstick somebody else\'s token', () => {
    const { system, prompt } = moonCaptionPrompt(
      { persona: 'PERSONA', botUsername: 'TinyMascot', today: '2026-09-14' },
      {
        token: tiny,
        target: rung(tiny, board[4]!),
        ladder: ladder(tiny, board, board[4]!, 3),
        boardName: 'AI meme coins',
        boardSize: 12,
        history: 'Yesterday: $TURBO at 127x',
        community: '',
        maxEmoji: 1,
        maxLength: 280,
      },
    )
    expect(system).toContain('PERSONA')
    expect(system).toContain('@TinyMascot')
    expect(prompt).toContain('$TINY with the market cap of $DRB')
    expect(prompt).toContain('37x')
    expect(prompt).toContain('#9 of 12 on the board')
    expect(prompt).toContain('#5 on the board')
    expect(prompt).toContain('not a forecast')
    expect(prompt).toContain("$DRB is somebody else's token")
    expect(prompt).toContain('Yesterday: $TURBO at 127x')
    expect(prompt).toContain('At most 1 emoji')
  })
})

function pipelineOptions(overrides: Partial<MoonPipelineOptions> = {}): MoonPipelineOptions & {
  x: XTools & { uploadMedia: ReturnType<typeof vi.fn>; createPost: ReturnType<typeof vi.fn> }
  memory: Memory & { captureActivity: ReturnType<typeof vi.fn>; recall: ReturnType<typeof vi.fn> }
} {
  const source: LeaderboardSource = {
    coin: vi.fn().mockResolvedValue(tiny),
    category: vi.fn().mockResolvedValue(board),
    logo: vi.fn().mockResolvedValue(undefined),
  }
  const text: TextModel = {
    complete: vi.fn().mockResolvedValue('{"text":"$TINY at $DRB\'s cap is 37x. Small robot, big spreadsheet."}'),
  }
  const memory = {
    captureActivity: vi.fn().mockResolvedValue(undefined),
    captureMention: vi.fn(),
    recall: vi.fn().mockResolvedValue(''),
    captureReply: vi.fn(),
  }
  const x = {
    searchPosts: vi.fn(),
    getPost: vi.fn(),
    getConversation: vi.fn(),
    createPost: vi.fn().mockResolvedValue({ id: 'post-1' }),
    uploadMedia: vi.fn().mockResolvedValue({ mediaId: 'media-1' }),
  }
  return {
    source,
    text,
    memory,
    x,
    persona: 'PERSONA',
    botUsername: 'TinyMascot',
    dryRun: false,
    coinId: 'tiny-humans-ai',
    category: 'ai-meme-coins',
    boardName: 'AI meme coins',
    boardSize: 30,
    rungs: 7,
    minMultiple: 2,
    maxMultiple: 100,
    maxEmoji: 1,
    random: () => 0.5,
    now: () => Date.UTC(2026, 8, 14, 20),
    ...overrides,
  }
}

describe('the pipeline', () => {
  it('reads the board, finds itself on it, picks, draws, captions, posts with the picture, and remembers the yardstick', async () => {
    const options = pipelineOptions()
    const result = await new MoonPipeline(options).run()
    expect(result.outcome).toBe('posted')
    expect(result.token.boardRank).toBe(9)
    expect(options.source.coin).not.toHaveBeenCalled()
    expect(result.postedId).toBe('post-1')
    expect(result.target.multiple).toBeGreaterThanOrEqual(2)
    expect(result.target.multiple).toBeLessThanOrEqual(100)
    expect(result.png.subarray(0, 4)).toEqual(Buffer.from([0x89, 0x50, 0x4e, 0x47]))
    expect(options.x.uploadMedia).toHaveBeenCalledWith({ data: result.png, mimeType: 'image/png' })
    expect(options.x.createPost).toHaveBeenCalledWith(result.caption, { mediaIds: ['media-1'] })
    expect(options.memory.captureActivity).toHaveBeenCalledWith(
      'moon_post',
      'post-1',
      expect.objectContaining({ yardstick: result.target.coin.symbol, boardRank: 9, published: true, text: result.caption }),
    )
    expect(result.svg).toContain(`>$${result.target.coin.symbol}<`)
  })

  it('falls back to the coin lookup when the token is not on the board, and says so', async () => {
    const options = pipelineOptions()
    ;(options.source.category as ReturnType<typeof vi.fn>).mockResolvedValue(board.filter((entry) => entry.id !== tiny.id))
    const result = await new MoonPipeline(options).run()
    expect(options.source.coin).toHaveBeenCalledWith('tiny-humans-ai')
    expect(result.problems.some((problem) => problem.includes('is not on the AI meme coins board'))).toBe(true)
    expect(result.outcome).toBe('posted')
  })

  it('never uses a struck-off coin or an unprintable ticker as the yardstick or a rung, but keeps their board places', async () => {
    const options = pipelineOptions({ random: () => 0, excludeIds: ['aic'] })
    ;(options.source.category as ReturnType<typeof vi.fn>).mockResolvedValue([coin('\u3000', 2_000_000, { id: 'blank', boardRank: 0 }), ...board])
    const result = await new MoonPipeline(options).run()
    expect(result.target.coin.symbol).toBe('DRB') // AIC would have been first
    const symbols = result.ladder.map((entry) => entry.coin.symbol)
    expect(symbols).not.toContain('AIC')
    expect(symbols).not.toContain('\u3000')
    expect(result.board.some((entry) => entry.id === 'aic')).toBe(true)
    expect(printable('e/acc')).toBe(false)
    expect(printable('q*')).toBe(false)
    expect(printable('FARTCOIN')).toBe(true)
    expect(printable('AI16Z')).toBe(true)
  })

  it('skips a yardstick memory says was posted lately', async () => {
    const options = pipelineOptions({ random: () => 0 })
    // With random 0 the pick is the smallest in band: AIC at 64x.
    expect((await new MoonPipeline(options).run()).target.coin.symbol).toBe('AIC')
    options.memory.recall.mockImplementation(async (query: string) =>
      query.includes('moon-chart') ? 'Monday: $AIC at 64x, joke about spreadsheets.' : '',
    )
    const result = await new MoonPipeline(options).run()
    expect(result.excluded).toEqual(['AIC'])
    expect(result.target.coin.symbol).toBe('DRB')
  })

  it('dry-runs without touching X and still remembers, labelled unpublished', async () => {
    const options = pipelineOptions({ dryRun: true })
    const result = await new MoonPipeline(options).run()
    expect(result.outcome).toBe('dry-run')
    expect(options.x.uploadMedia).not.toHaveBeenCalled()
    expect(options.x.createPost).not.toHaveBeenCalled()
    expect(options.memory.captureActivity).toHaveBeenCalledWith('moon_post', expect.stringMatching(/^dry-run:/), expect.objectContaining({ published: false }))
  })

  it('holds a caption the filter refuses and posts nothing', async () => {
    const options = pipelineOptions({
      text: { complete: vi.fn().mockResolvedValue('{"text":"$TINY at 37x, go kill the shorts"}') },
      safety: lexicalGate(),
    })
    const result = await new MoonPipeline(options).run()
    expect(result.outcome).toBe('held')
    expect(result.caption).toBe('')
    expect(options.x.createPost).not.toHaveBeenCalled()
    expect(result.problems.some((problem) => problem.startsWith('content filter withheld'))).toBe(true)
  })

  it('cuts a link from the caption rather than posting it', async () => {
    const options = pipelineOptions({
      text: { complete: vi.fn().mockResolvedValue('{"text":"$TINY at 37x, see https://evil.example/now"}') },
      safety: lexicalGate(),
    })
    const result = await new MoonPipeline(options).run()
    expect(result.outcome).toBe('posted')
    expect(result.caption).not.toContain('evil.example')
    expect(result.problems.some((problem) => problem.startsWith('the content filter edited'))).toBe(true)
  })

  it('sends an over-long caption back once', async () => {
    const complete = vi
      .fn()
      .mockResolvedValueOnce(`{"text":"${'x'.repeat(300)}"}`)
      .mockResolvedValueOnce('{"text":"short"}')
    const result = await new MoonPipeline(pipelineOptions({ text: { complete } })).run()
    expect(complete).toHaveBeenCalledTimes(2)
    expect(result.caption).toBe('short')
  })

  it('fails plainly when nothing on the board is above the token', async () => {
    const options = pipelineOptions()
    ;(options.source.category as ReturnType<typeof vi.fn>).mockResolvedValue([coin('BELOW', 100_000)])
    await expect(new MoonPipeline(options).run()).rejects.toThrow(/no moon to draw/)
  })
})

describe('the job', () => {
  function result(): MoonResult {
    return {
      token: tiny,
      board,
      target: rung(tiny, board[4]!),
      ladder: ladder(tiny, board, board[4]!, 3),
      excluded: ['AIC'],
      svg: '<svg/>',
      png: Buffer.from(PNG),
      caption: 'the caption',
      outcome: 'posted',
      postedId: 'post-1',
      problems: [],
      recalled: { history: 'h', community: 'c' },
    }
  }

  it('refuses to run twice inside the interval, and journals the turn', async () => {
    const pipeline = { run: vi.fn().mockResolvedValue(result()) }
    const journal = { record: vi.fn().mockResolvedValue(undefined) } as unknown as Journal
    let now = Date.UTC(2026, 8, 14, 20)
    const job = new MoonJob({ pipeline, logger: { info: vi.fn(), error: vi.fn() }, journal, now: () => now })
    expect(await job.run()).toBeDefined()
    now += 60 * 60 * 1_000
    expect(await job.run()).toBeUndefined()
    expect(pipeline.run).toHaveBeenCalledTimes(1)
    expect(journal.record).toHaveBeenCalledWith(
      expect.objectContaining({ id: 'moon:2026-09-14', trigger: 'moon', said: 'the caption', outcome: 'posted' }),
    )
    const entry = (journal.record as ReturnType<typeof vi.fn>).mock.calls[0]![0] as { thinking: string }
    expect(entry.thinking).toContain('yardstick: $DRB')
    expect(entry.thinking).toContain('skipped as recently used: AIC')
  })

  it('journals a failure and rethrows', async () => {
    const pipeline = { run: vi.fn().mockRejectedValue(new Error('gecko down')) }
    const journal = { record: vi.fn().mockResolvedValue(undefined) } as unknown as Journal
    const job = new MoonJob({ pipeline, logger: { info: vi.fn(), error: vi.fn() }, journal })
    await expect(job.run()).rejects.toThrow('gecko down')
    expect(journal.record).toHaveBeenCalledWith(expect.objectContaining({ trigger: 'moon', outcome: 'failed', outcomeNote: 'gecko down' }))
  })
})

describe('the rota', () => {
  const rota = { days: 3, anchor: '2026-09-14' }
  const day = (date: string, hour = 20): number => Date.UTC(...(date.split('-').map(Number) as [number, number, number]).map((v, i) => (i === 1 ? v - 1 : v)) as [number, number, number], hour)

  it('counts slots from the anchor and wraps across a month end', async () => {
    const { rotaSlot, isTurn } = await import('../src/jobs/cycle.js')
    expect(rotaSlot(rota, day('2026-09-14'))).toBe(0)
    expect(rotaSlot(rota, day('2026-09-15'))).toBe(1)
    expect(rotaSlot(rota, day('2026-09-16'))).toBe(2)
    expect(rotaSlot(rota, day('2026-09-17'))).toBe(0)
    expect(rotaSlot(rota, day('2026-09-30'))).toBe(1)
    expect(rotaSlot(rota, day('2026-10-01'))).toBe(2)
    expect(rotaSlot(rota, day('2026-10-02'))).toBe(0)
    // Before the anchor still lands on a real slot.
    expect(rotaSlot(rota, day('2026-09-13'))).toBe(2)
    expect(isTurn(rota, 1, day('2026-09-15'))).toBe(true)
    expect(isTurn(rota, 5, day('2026-09-15'))).toBe(false)
    expect(() => rotaSlot({ days: 3, anchor: 'yesterday' }, 0)).toThrow(/ISO date/)
  })

  it('keeps the moon to its day and the chart to the next, with the third quiet', async () => {
    const { ChartJob } = await import('../src/jobs/chart-job.js')
    const moonPipeline = { run: vi.fn().mockResolvedValue(undefined as unknown as MoonResult) }
    const chartAgent = { run: vi.fn().mockResolvedValue({ output: '', steps: [] }) }
    let now = day('2026-09-14')
    const logger = { info: vi.fn(), error: vi.fn() }
    const moon = new MoonJob({ pipeline: moonPipeline, logger, now: () => now, rota: { rota, slot: 0 } })
    const chart = new ChartJob({ agent: chartAgent, symbol: 'TINY', timeframe: '1h', logger, now: () => now, rota: { rota, slot: 1 } })
    moonPipeline.run.mockImplementation(async () => ({ outcome: 'dry-run', target: { coin: { symbol: 'X' }, multiple: 2 }, problems: [] }) as unknown as MoonResult)

    await moon.run()
    await chart.run()
    expect(moonPipeline.run).toHaveBeenCalledTimes(1)
    expect(chartAgent.run).not.toHaveBeenCalled()

    now = day('2026-09-15')
    await moon.run()
    await chart.run()
    expect(moonPipeline.run).toHaveBeenCalledTimes(1)
    expect(chartAgent.run).toHaveBeenCalledTimes(1)

    now = day('2026-09-16')
    await moon.run()
    await chart.run()
    expect(moonPipeline.run).toHaveBeenCalledTimes(1)
    expect(chartAgent.run).toHaveBeenCalledTimes(1)

    // A forced run (the rehearsal scripts) ignores the rota.
    expect(await chart.run({ force: true })).toBe(true)
  })
})
