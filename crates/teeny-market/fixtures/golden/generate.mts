// Golden generator for teeny-market's parity tests: runs the legacy
// TypeScript on fixed inputs and records the inputs with what it returned.
// Not part of the build. Regenerate from the repo root with:
//   (cd legacy && ./node_modules/.bin/tsx ../crates/teeny-market/fixtures/golden/generate.mts)
import { writeFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
const L = fileURLToPath(new URL('../../../../legacy/src', import.meta.url))
const OUT = fileURLToPath(new URL('.', import.meta.url))
const { compactUsd, price, signedPercent } = await import(`${L}/chart/format.ts`)
const ind = await import(`${L}/ta/indicators.ts`)
const { analyse } = await import(`${L}/ta/analysis.ts`)
const { readProgress } = await import(`${L}/ta/progress.ts`)
const { relativeStrength } = await import(`${L}/market/global.ts`)
const { TokenMarket } = await import(`${L}/market/market.ts`)
const { createMarketTools } = await import(`${L}/agent/tools/market.ts`)
const { createCommunityTools } = await import(`${L}/agent/tools/community.ts`)
const { createNewsTools } = await import(`${L}/agent/tools/news.ts`)

const write = (name: string, value: unknown) => writeFileSync(`${OUT}/${name}.json`, JSON.stringify(value) + '\n')

// ---- format -----------------------------------------------------------------
const values = [0, 0.000747, 0.0007472834089, 0.00000001234, 0.0000001, 0.5, 1, 1.23456, 12.3456, 99.99, 100,
  999, 999.4, 999.96, 1000, 1234.5678, 9999.999, 730709.30528831, 93428, 10134, 1e6, 1_240_000, 12_300_000_000,
  -3657, -0.004, -1.25, 2699012632815, 78327, 103.33, 0.125, 2.5, 1.005, 1.0005, 0.045, 58.3, 17.38, 1e21, 123456789.123,
  0.1 + 0.2, 4.35, 8.345, 1.45, -0.0000003]
write('format', values.map((v) => ({
  value: v, compactUsd: compactUsd(v), price: price(v), signedPercent: signedPercent(v), string: String(v),
  fixed0: v.toFixed(0), fixed1: v.toFixed(1), fixed2: v.toFixed(2), precision4: v.toPrecision(4),
  locale: v.toLocaleString('en-US'), locale2: v.toLocaleString('en-US', { maximumFractionDigits: 2 }),
  round: Math.round(v),
})))

// ---- series -----------------------------------------------------------------
function lcg(seed: number) { let s = seed; return () => { s = (s * 1103515245 + 12345) % 2147483648; return s / 2147483648 } }
function walk(n: number, start: number, vol: number, seed: number, step = 3600, drift = 0) {
  const r = lcg(seed); let p = start; const out = []
  for (let i = 0; i < n; i++) {
    const o = p; const c = o * (1 + (r() - 0.5 + drift) * vol)
    out.push({ time: 1_700_000_000 + i * step, open: o, high: Math.max(o, c) * (1 + r() * vol / 2),
      low: Math.min(o, c) * (1 - r() * vol / 2), close: c, volume: r() * 5000 })
    p = c
  }
  return out
}
function fromCloses(closes: number[], volumes?: number[], wick = 0.005) {
  return closes.map((close, index) => {
    const open = index === 0 ? close : closes[index - 1]!
    return { time: 1_700_000_000 + index * 3_600, open, high: Math.max(open, close) * (1 + wick),
      low: Math.min(open, close) * (1 - wick), close, volume: volumes?.[index] ?? 100 }
  })
}
const rising = Array.from({ length: 120 }, (_, i) => 100 + i * 1.5)
const falling = Array.from({ length: 120 }, (_, i) => 300 - i * 1.5)
const withLevel = [...Array.from({ length: 20 }, (_, i) => 90 + i * 0.2), 100, 94, 100, 93, 99.8, 92, 100.2, 95, ...Array.from({ length: 12 }, (_, i) => 96 + i * 0.6)]
const withSupport = [...Array.from({ length: 20 }, (_, i) => 96 + i * 0.2), 100, 94, 100.1, 93.9, 99.9, 94.1, 100, 94, ...Array.from({ length: 20 }, (_, i) => 101 + i * 0.5)]
const broken = [...Array.from({ length: 20 }, (_, i) => 100 + i * 0.1), 102, 98, 102, 97.5, 101.8, 98, 102, 97, 92, 90.5, 91, 90.8, 91.2, 91, 91.4, 91.1, 91.6]
const series: Record<string, any[]> = {
  rising: fromCloses(rising), falling: fromCloses(falling),
  bounce: fromCloses([...falling.slice(0, 112), 140, 143, 147, 152, 158, 165, 173, 182]),
  spike: fromCloses(rising, rising.map((_, i) => (i === 119 ? 5_000 : 100))),
  waking: fromCloses(rising, rising.map((_, i) => (i < 60 ? 100 : 400))),
  drying: fromCloses(rising, rising.map((_, i) => (i < 60 ? 400 : 100))),
  levelLoud: fromCloses(withLevel, withLevel.map((_, i) => (i >= withLevel.length - 3 ? 900 : 100))),
  levelQuiet: fromCloses(withLevel),
  withSupport: fromCloses(withSupport), broken: fromCloses(broken),
  brokenFalling: fromCloses([...broken.slice(0, 28), 92, 90, 88, 86, 84, 82, 80]),
  justBroke: fromCloses([...broken.slice(0, 28), 92]),
  flat: fromCloses(Array.from({ length: 40 }, () => 1)),
  three: fromCloses([1, 2, 3]),
  memeWalk: walk(240, 0.0007, 0.06, 7), memeUp: walk(160, 0.0004, 0.05, 11, 3600, 0.08),
  memeDown: walk(160, 0.002, 0.05, 23, 3600, -0.08), zeroVolume: walk(60, 5, 0.02, 3).map((c) => ({ ...c, volume: 0 })),
}
write('indicators', Object.entries(series).filter(([k]) => ['memeWalk', 'rising', 'levelLoud', 'flat', 'zeroVolume'].includes(k)).map(([name, candles]) => {
  const closes = ind.closes(candles)
  return { name, candles, sma20: ind.sma(closes, 20), ema20: ind.ema(closes, 20), ema50: ind.ema(closes, 50),
    bollinger: ind.bollinger(closes, 20, 2), atr14: ind.atr(candles, 14), vwap: ind.vwap(candles) ?? null,
    swings: ind.swings(candles, 2), levels: ind.levels(candles), levelsLoose: ind.levels(candles, { tolerance: 0.02, span: 3 }) }
}))
write('analysis', Object.entries(series).map(([name, candles]) => ({ name, candles, timeframe: '1h', read: analyse(candles, { timeframe: '1h' }) })))

const daily = (closes: number[]) => fromCloses(closes, undefined, 0.02).map((c, i) => ({ ...c, time: 1_700_000_000 + i * 86_400, volume: 1000 }))
const progressCases: Record<string, any[]> = {
  long: daily(Array.from({ length: 100 }, (_, i) => 1 + i * 0.1)), young: daily([1, 1.2, 1.4, 1.8]),
  stepping: daily([20, 14, 20, 26, 18, 26, 32, 24, 32, 38, 30, 38]), notStepping: daily([20, 18, 19, 15, 16, 12, 13, 9, 10]),
  redDay: daily([1, 2, 4, 8, 16, 32, 30, 28]), allDown: daily(Array.from({ length: 40 }, (_, i) => 100 - i * 2)),
  range: daily([10, 20, 15]), meme: walk(130, 0.0007, 0.12, 5, 86_400),
}
write('progress', Object.entries(progressCases).map(([name, candles]) => ({ name, candles, progress: readProgress(candles) })))

// ---- relative strength --------------------------------------------------------
const down = { totalMarketCapUsd: 2_699_012_632_815, totalChange24hPct: -1.8, btcDominancePct: 58.3,
  assets: [{ symbol: 'BTC', priceUsd: 78_327, change24hPct: -1.11 }, { symbol: 'ETH', priceUsd: 2412.07, change24hPct: -2.456 }, { symbol: 'SOL', priceUsd: 103.33, change24hPct: -0.61 }], source: 'coingecko' }
const up = { totalChange24hPct: 40, assets: [{ symbol: 'BTC', priceUsd: 78_327, change24hPct: 30 }], source: 'coingecko' }
const flatMkt = { ...down, totalChange24hPct: 0.1 }
const noTotal = { assets: [{ symbol: 'BTC', priceUsd: 64_000.5 }, { symbol: 'SOL', priceUsd: 0.9876, change24hPct: 0.44 }], source: 'coingecko' }
write('strength', [[14.2, down], [-9, down], [2, flatMkt], [0.33, up], [-0.15, noTotal], [1.05, noTotal]].map(([change, snapshot]) => ({ change, snapshot, strength: relativeStrength(change as number, snapshot as any) })))

// ---- tools ------------------------------------------------------------------
const ADDRESS = '2AF7CqwieUjUPALL7icuZtL3X7wENdjUjGBMmfV2pump'
const W = (n: string) => `${n}${'x'.repeat(40 - n.length)}`
const trades = [
  ['t1', '2026-09-08T10:00:00Z', 'buy', W('Alpha'), 512.4, 700000, 0.00073],
  ['t2', '2026-09-08T10:05:00Z', 'buy', W('Alpha'), 1_088.25, 1400000, 0.00074],
  ['t3', '2026-09-08T09:15:00Z', 'sell', W('Bravo'), 2_401.9, 3300000, 0.00072],
  ['t4', '2026-09-08T11:20:00Z', 'buy', W('KolOne'), 15_210.7, 20000000, 0.00076],
  ['t5', '2026-09-08T11:40:00Z', 'sell', W('kolone'), 300, 400000, 0.00075],
  ['t6', '2026-09-08T12:00:00Z', 'buy', W('Charlie'), 99.5, 130000, 0.00076],
  ['t7', '2026-09-08T12:30:00Z', 'sell', W('Delta'), 150, 200000, 0.00075],
  ['t8', '2026-09-08T12:45:00Z', 'buy', W('Echo'), 250, 330000, 0.00075],
  ['t9', '2026-09-08T12:50:00Z', 'buy', W('Fox'), 175, 230000, 0.00075],
  ['t10', '2026-09-08T12:55:00Z', 'buy', W('Golf'), 180, 240000, 0.00075],
  ['t11', '2026-09-08T13:00:00Z', 'sell', 'Short', 120, 160000, 0.00075],
].map(([txHash, timestamp, kind, wallet, volumeUsd, tokenAmount, priceUsd]) => ({ txHash, timestamp, kind, wallet, volumeUsd, tokenAmount, priceUsd }))
const scenarios: any[] = [
  { name: 'tiny', symbol: 'TINY', watchlist: [{ wallet: W('KOLONE'), label: 'a known KOL' }],
    overview: { address: ADDRESS, chain: 'solana', name: 'Tiny Humans AI', symbol: 'TINY', priceUsd: 0.000747, marketCapUsd: 730_709.30528831,
      fdvUsd: 747_000.1, liquidityUsd: 93_428, volume24hUsd: 10_134, priceChange: { m5: 0.4, h1: -1.25, h6: 3.051, h24: 17.38 },
      txns24h: { buys: 79, sells: 75 }, poolAddress: 'GEF9pool', poolName: 'raydium', pairCreatedAt: '2025-01-02T03:04:05.000Z', source: 'dexscreener+geckoterminal' },
    candles: { '1h': walk(240, 0.0007, 0.04, 101, 3600, 0.01), '4h': walk(200, 0.0005, 0.06, 102, 14400, 0.02),
      '1d': walk(130, 0.0003, 0.12, 103, 86400, 0.03), '1w': walk(30, 0.0002, 0.25, 104, 604800, 0.04) },
    holders: { totalSupply: 999_870_123.456, decimals: 6, top10Share: 0.4123456, source: 'solana-rpc',
      top: Array.from({ length: 12 }, (_, i) => ({ address: W(`Acct${i}`), amount: 1e8 / (i + 1), share: 0.1 / (i + 1) })) },
    ranked: [{ wallet: W('Smart'), realizedProfitUsd: 12_345.6, boughtUsd: 40_000, soldUsd: 52_345.6, tags: ['smart_money', 'kol'] },
      { wallet: W('Whale'), tags: [] }, { wallet: 'tiny', boughtUsd: 999, tags: ['whale'] }],
    global: down, traderWallets: [W('alpha'), W('Nobody')] },
  { name: 'falling', symbol: undefined, watchlist: [],
    overview: { address: ADDRESS, chain: 'solana', priceUsd: 1234.5678, priceChange: {}, source: 'geckoterminal' },
    candles: { '1h': walk(130, 0.002, 0.03, 201, 3600, -0.06), '4h': walk(130, 0.004, 0.04, 202, 14400, -0.06),
      '1d': walk(130, 0.009, 0.08, 203, 86400, -0.05), '1w': walk(30, 0.02, 0.2, 204, 604800, -0.06) },
    holdersError: 'Solana RPC getTokenLargestAccounts answered 429', ranked: [], global: up, traderWallets: [W('Bravo')], trades: [] },
  { name: 'young', symbol: 'NEW', watchlist: [],
    overview: { address: ADDRESS, chain: 'solana', symbol: 'NEW', priceUsd: 0.00000001234, marketCapUsd: 999, priceChange: { h1: 0 }, source: 'gmgn+dexscreener' },
    candles: { '1h': walk(15, 0.01, 0.05, 301), '4h': walk(4, 0.01, 0.05, 302, 14400), '1d': walk(1, 0.01, 0.05, 303, 86400), '1w': [] },
    holders: { totalSupply: 1000, decimals: 0, top10Share: 1, source: 'solana-rpc', top: [{ address: 'Solo', amount: 1000, share: 1 }] },
    ranked: [], global: undefined, traderWallets: [] },
  { name: 'noTokenChange', symbol: 'TINY', watchlist: [],
    overview: { address: ADDRESS, chain: 'solana', priceUsd: 0.5, priceChange: { h1: 2 }, source: 'dexscreener' },
    candles: { '1h': walk(60, 0.5, 0.02, 401), '4h': walk(60, 0.5, 0.02, 402, 14400), '1d': walk(60, 0.5, 0.02, 403, 86400), '1w': walk(10, 0.5, 0.02, 404, 604800) },
    holders: undefined, holdersError: 'Holder data is only wired up for Solana', ranked: [], global: noTotal, traderWallets: [] },
]
const calls = [
  ['token_overview', {}], ['market_context', {}], ['token_candles', { timeframe: '1h', bars: 120 }], ['token_candles', { timeframe: '1d', bars: 30 }],
  ['technical_analysis', { timeframe: '1h', bars: 120 }], ['technical_analysis', { timeframe: '4h', bars: 60 }],
  ['chart_timeframes', {}], ['token_progress', { days: 120 }], ['token_progress', { days: 7 }], ['token_holders', {}],
  ['token_flow', { minVolumeUsd: 100 }], ['token_flow', { minVolumeUsd: 0 }], ['ranked_traders', {}],
] as Array<[string, Record<string, unknown>]>
const toolCases = []
for (const s of scenarios) {
  const tradeList = s.trades ?? trades
  const market = Object.assign(new TokenMarket({ watchlist: s.watchlist }), {
    overview: async () => s.overview,
    candles: async ({ timeframe, limit }: any) => (s.candles[timeframe] ?? []).slice(-limit),
    holders: async () => { if (s.holdersError) throw new Error(s.holdersError); return s.holders },
    rankedTraders: async () => s.ranked,
    trades: async ({ minVolumeUsd }: any) => tradeList.filter((t: any) => t.volumeUsd > Math.max(minVolumeUsd ?? 0, 0)),
  })
  const tools = new Map(createMarketTools({ market, address: ADDRESS, ...(s.symbol ? { symbol: s.symbol } : {}), x: {} as any, dryRun: true,
    ...(s.global ? { global: { snapshot: async () => s.global } as any } : {}) }).map((t: any) => [t.name, t]))
  const outputs = []
  for (const [name, input] of calls) outputs.push({ tool: name, input, output: await tools.get(name).invoke(input) })
  for (const wallet of s.traderWallets) for (const minVolumeUsd of [0, 200]) {
    const input = { wallet, minVolumeUsd }; outputs.push({ tool: 'trader_activity', input, output: await tools.get('trader_activity').invoke(input) })
  }
  toolCases.push({ scenario: { ...s, trades: tradeList, traderWallets: undefined }, outputs })
}
write('tools', toolCases)

// ---- community + news ---------------------------------------------------------
const posts = [
  { id: '1', text: 'still holding $TINY\n\n  through   all of this', authorUsername: 'aholder', createdAt: '2026-09-09T10:00:00Z' },
  { id: '2', text: 'my own post', authorUsername: 'TeenyDroid' },
  { id: '3', text: 'no author here' },
]
const community = []
for (const [label, result, maxResults, searchLimit] of [['some', posts, 10, undefined], ['capped', posts, 50, 25], ['own only', [posts[1]], 10, undefined], ['empty', [], 10, undefined], ['down', new Error('X search answered 429'), 10, undefined]] as any[]) {
  let seen: any
  const x = { searchPosts: async (q: string, o: any) => { seen = { query: q, ...o }; if (result instanceof Error) throw result; return result } }
  const [pulse] = createCommunityTools({ x: x as any, query: '$TINY -is:retweet', botUsername: '@teenydroid', ...(searchLimit ? { searchLimit } : {}) })
  const output = await pulse!.invoke({ maxResults })
  community.push({ label, posts: result instanceof Error ? undefined : result, error: result instanceof Error ? result.message : undefined, maxResults, searchLimit, searched: seen, output })
}
write('community', community)
const hits = [
  { title: 'Bitcoin slips as ETF flows cool', url: 'https://a', publishedDate: '2026-09-08T10:00:00.000Z', highlights: ['Bitcoin fell 2%.', 'Flows cooled.'] },
  { title: null, url: 'https://b', text: `  ${'long text '.repeat(60)}  ` },
  { title: 'Empty', url: 'https://c', publishedDate: null, highlights: [] },
  { title: 'Only blank text', url: 'https://d', text: '    ' },
]
const news = []
for (const [label, result, input, defaultTopic] of [['hits', hits, { days: 3, numResults: 5 }, undefined], ['topic', hits.slice(0, 1), { topic: '  solana memecoins ', days: 1, numResults: 2 }, 'ignored'],
  ['default topic', [], { topic: '   ', days: 14, numResults: 10 }, 'ai agents'], ['down', new Error('exa down'), { days: 3, numResults: 5 }, undefined]] as any[]) {
  let seen: any
  const exa = { searchAndContents: async (q: string, o: any) => { seen = { query: q, ...o, startPublishedDate: undefined }; if (result instanceof Error) throw result; return { results: result } } }
  const [tool] = createNewsTools(exa as any, defaultTopic ? { defaultTopic } : {})
  const output = await tool!.invoke(input)
  news.push({ label, hits: result instanceof Error ? undefined : result, error: result instanceof Error ? result.message : undefined, input, defaultTopic, searched: seen, output })
}
write('news', news)
console.log('ok')
