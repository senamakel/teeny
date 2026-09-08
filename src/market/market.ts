import { DexscreenerClient } from './dexscreener.js'
import { GeckoTerminalClient } from './geckoterminal.js'
import { GmgnClient, type RankedTrader } from './gmgn.js'
import { SolanaRpc } from './solana.js'
import type { Candle, HolderSnapshot, Timeframe, TokenOverview, Trade } from './types.js'

/** A wallet worth naming: a KOL, a fund, a team wallet, a known exit liquidity enjoyer. */
export interface LabelledWallet {
  wallet: string
  label: string
}

export interface WalletActivity {
  wallet: string
  label?: string
  buys: number
  sells: number
  boughtUsd: number
  soldUsd: number
  /** Bought minus sold, in the window read. Positive is accumulation. */
  netUsd: number
  firstSeen: string
  lastSeen: string
}

export interface FlowSummary {
  /** How far back the trades read actually go. Everything here is about this window only. */
  window: { from: string; to: string; trades: number }
  buyVolumeUsd: number
  sellVolumeUsd: number
  netUsd: number
  uniqueWallets: number
  buyers: number
  sellers: number
  topBuyers: WalletActivity[]
  topSellers: WalletActivity[]
  /** Watchlist wallets seen trading in the window. */
  known: WalletActivity[]
}

interface Cached<T> {
  at: number
  value: T
}

/**
 * One place to ask about a token.
 *
 * Three providers sit behind it because none of them answers everything:
 * Dexscreener has the best snapshot and no candles, GeckoTerminal has candles
 * and per-trade wallets, and an RPC has supply and concentration. GMGN would
 * have all of it and is unreachable from a server, so it is tried first only
 * when it has been given a base URL that works.
 *
 * Every method degrades rather than throwing where it sensibly can: a chart
 * post that loses the holder breakdown is still a chart post, and the agent is
 * told what is missing rather than being handed a plausible-looking zero.
 */
export class TokenMarket {
  private readonly gecko: GeckoTerminalClient
  private readonly dexscreener: DexscreenerClient
  private readonly solana?: SolanaRpc
  private readonly gmgn?: GmgnClient
  private readonly watchlist: Map<string, string>
  private readonly ttlMs: number
  private readonly overviews = new Map<string, Cached<TokenOverview>>()

  constructor(
    options: {
      network?: string
      solanaRpcUrl?: string
      gmgnBaseUrl?: string
      /** Wallets worth naming when they show up in the flow. */
      watchlist?: LabelledWallet[]
      /** How long a snapshot may be reused. A daily post does not need a live tick. */
      cacheTtlMs?: number
      timeoutMs?: number
    } = {},
  ) {
    const network = options.network ?? 'solana'
    this.gecko = new GeckoTerminalClient({
      network,
      ...(options.timeoutMs === undefined ? {} : { timeoutMs: options.timeoutMs }),
    })
    this.dexscreener = new DexscreenerClient(
      options.timeoutMs === undefined ? {} : { timeoutMs: options.timeoutMs },
    )
    if (network === 'solana') {
      this.solana = new SolanaRpc({
        ...(options.solanaRpcUrl ? { url: options.solanaRpcUrl } : {}),
        ...(options.timeoutMs === undefined ? {} : { timeoutMs: options.timeoutMs }),
      })
    }
    if (options.gmgnBaseUrl) {
      this.gmgn = new GmgnClient({
        baseUrl: options.gmgnBaseUrl,
        ...(options.timeoutMs === undefined ? {} : { timeoutMs: options.timeoutMs }),
      })
    }
    this.watchlist = new Map(
      (options.watchlist ?? []).map((entry) => [entry.wallet.toLowerCase(), entry.label]),
    )
    this.ttlMs = options.cacheTtlMs ?? 60_000
  }

  /** Whether GMGN is wired up, which the agent is told so it never claims a source it lacks. */
  get sources(): string[] {
    return [
      ...(this.gmgn ? ['gmgn'] : []),
      'dexscreener',
      'geckoterminal',
      ...(this.solana ? ['solana-rpc'] : []),
    ]
  }

  /**
   * The snapshot, from whichever provider answers first, filled in from the
   * others.
   *
   * Merged rather than picked because the gaps do not overlap: Dexscreener has
   * five-minute and one-hour change and no reliable pool id for GeckoTerminal's
   * OHLCV path, and GeckoTerminal has market cap where Dexscreener sends only
   * FDV. Taking one and discarding the other loses a field the post wanted.
   */
  async overview(address: string): Promise<TokenOverview> {
    const cached = this.overviews.get(address)
    if (cached && Date.now() - cached.at < this.ttlMs) return cached.value

    const attempts = await Promise.allSettled([
      this.gmgn ? this.gmgn.overview(address) : Promise.reject(new Error('gmgn not configured')),
      this.dexscreener.overview(address),
      this.gecko.overview(address),
    ])
    const answered = attempts.flatMap((attempt) =>
      attempt.status === 'fulfilled' ? [attempt.value] : [],
    )
    const primary = answered[0]
    if (!primary) {
      const why = attempts
        .flatMap((attempt) =>
          attempt.status === 'rejected' ? [String((attempt.reason as Error)?.message ?? attempt.reason)] : [],
        )
        .join('; ')
      throw new Error(`No market source could price ${address}: ${why}`)
    }

    const merged = answered.slice(1).reduce<TokenOverview>((into, extra) => fill(into, extra), primary)
    this.overviews.set(address, { at: Date.now(), value: merged })
    return merged
  }

  /** The pool a chart should be drawn from: the deepest one anybody reported. */
  async pool(address: string): Promise<string> {
    const overview = await this.overview(address)
    if (!overview.poolAddress) throw new Error(`No pool known for ${address}`)
    return overview.poolAddress
  }

  async candles(input: {
    address: string
    timeframe: Timeframe
    limit?: number
  }): Promise<Candle[]> {
    if (this.gmgn) {
      const fromGmgn = await this.gmgn.candles(input).catch(() => [])
      if (fromGmgn.length > 0) return fromGmgn
    }
    return this.gecko.candles({
      pool: await this.pool(input.address),
      timeframe: input.timeframe,
      ...(input.limit === undefined ? {} : { limit: input.limit }),
    })
  }

  async trades(input: { address: string; minVolumeUsd?: number }): Promise<Trade[]> {
    return this.gecko.trades({
      pool: await this.pool(input.address),
      ...(input.minVolumeUsd === undefined ? {} : { minVolumeUsd: input.minVolumeUsd }),
    })
  }

  async holders(address: string): Promise<HolderSnapshot> {
    if (!this.solana) throw new Error('Holder data is only wired up for Solana')
    return this.solana.holders(address)
  }

  /** GMGN's ranked traders, when GMGN is reachable. Empty when it is not. */
  async rankedTraders(address: string): Promise<RankedTrader[]> {
    if (!this.gmgn) return []
    return this.gmgn.topTraders(address).catch(() => [])
  }

  /**
   * Who is doing what, aggregated from the recent trades page.
   *
   * This is a window, not a history: the provider returns the most recent few
   * hundred swaps over the size floor and nothing older. Every number here is
   * true of that window and says nothing about the day before it, which is why
   * the window is reported alongside the totals rather than left implicit.
   */
  async flow(input: { address: string; minVolumeUsd?: number }): Promise<FlowSummary> {
    const trades = await this.trades(input)
    return summariseFlow(trades, this.watchlist)
  }

  /** One wallet's footprint in the same window. */
  async wallet(input: { address: string; wallet: string; minVolumeUsd?: number }): Promise<WalletActivity | undefined> {
    const trades = await this.trades({
      address: input.address,
      ...(input.minVolumeUsd === undefined ? {} : { minVolumeUsd: input.minVolumeUsd }),
    })
    const target = input.wallet.toLowerCase()
    const mine = trades.filter((trade) => trade.wallet.toLowerCase() === target)
    if (mine.length === 0) return undefined
    return activityOf(input.wallet, mine, this.watchlist)
  }
}

export function summariseFlow(trades: Trade[], watchlist: Map<string, string>): FlowSummary {
  const byWallet = new Map<string, Trade[]>()
  for (const trade of trades) {
    const key = trade.wallet.toLowerCase()
    byWallet.set(key, [...(byWallet.get(key) ?? []), trade])
  }
  const wallets = [...byWallet.entries()].map(([, group]) =>
    activityOf(group[0]?.wallet ?? '', group, watchlist),
  )
  const times = trades.map((trade) => trade.timestamp).sort()
  const buyVolume = sum(trades.filter((trade) => trade.kind === 'buy').map((t) => t.volumeUsd))
  const sellVolume = sum(trades.filter((trade) => trade.kind === 'sell').map((t) => t.volumeUsd))

  return {
    window: {
      from: times[0] ?? '',
      to: times[times.length - 1] ?? '',
      trades: trades.length,
    },
    buyVolumeUsd: buyVolume,
    sellVolumeUsd: sellVolume,
    netUsd: buyVolume - sellVolume,
    uniqueWallets: wallets.length,
    buyers: wallets.filter((wallet) => wallet.buys > 0).length,
    sellers: wallets.filter((wallet) => wallet.sells > 0).length,
    topBuyers: [...wallets].sort((a, b) => b.boughtUsd - a.boughtUsd).filter((w) => w.boughtUsd > 0).slice(0, 5),
    topSellers: [...wallets].sort((a, b) => b.soldUsd - a.soldUsd).filter((w) => w.soldUsd > 0).slice(0, 5),
    known: wallets.filter((wallet) => wallet.label !== undefined),
  }
}

function activityOf(wallet: string, trades: Trade[], watchlist: Map<string, string>): WalletActivity {
  const times = trades.map((trade) => trade.timestamp).sort()
  const label = watchlist.get(wallet.toLowerCase())
  const bought = sum(trades.filter((trade) => trade.kind === 'buy').map((t) => t.volumeUsd))
  const sold = sum(trades.filter((trade) => trade.kind === 'sell').map((t) => t.volumeUsd))
  return {
    wallet,
    ...(label ? { label } : {}),
    buys: trades.filter((trade) => trade.kind === 'buy').length,
    sells: trades.filter((trade) => trade.kind === 'sell').length,
    boughtUsd: bought,
    soldUsd: sold,
    netUsd: bought - sold,
    firstSeen: times[0] ?? '',
    lastSeen: times[times.length - 1] ?? '',
  }
}

function sum(values: number[]): number {
  return values.reduce((total, value) => total + value, 0)
}

/** Copies across only the fields the first provider left blank. */
function fill(into: TokenOverview, extra: TokenOverview): TokenOverview {
  return {
    ...extra,
    ...into,
    priceChange: { ...extra.priceChange, ...into.priceChange },
    ...(into.txns24h ? { txns24h: into.txns24h } : extra.txns24h ? { txns24h: extra.txns24h } : {}),
    source: [into.source, extra.source].join('+'),
  }
}
