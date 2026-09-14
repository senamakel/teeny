import type { ListedCoin } from './leaderboard.js'

/** What the account's token would be at another coin's market cap. */
export interface Rung {
  coin: ListedCoin
  /** The target's market cap over the token's own. */
  multiple: number
  /** The token's price at that market cap, with its circulating supply as it is today. */
  impliedPriceUsd: number
}

/**
 * The arithmetic behind "$TINY with the market cap of X".
 *
 * Price at the target cap is the target's cap divided by the token's own
 * circulating supply, which is the same as today's price times the multiple.
 * It is a hypothetical about today's supply, not a forecast; the picture
 * says so and so does the caption.
 */
export function rung(token: ListedCoin, target: ListedCoin): Rung {
  return {
    coin: target,
    multiple: target.marketCapUsd / token.marketCapUsd,
    impliedPriceUsd: target.marketCapUsd / token.circulatingSupply,
  }
}

/** The name of the used-target list in memory, and what a coin appears as in it. */
export function usedSymbols(recalled: string, board: ListedCoin[]): Set<string> {
  const used = new Set<string>()
  if (!recalled.trim()) return used
  for (const coin of board) {
    // Whole-token match on the symbol, case-insensitive, with or without the
    // cashtag: "FARTCOIN", "$fartcoin" and "Fartcoin's cap" all count.
    const pattern = new RegExp(`(^|[^A-Za-z0-9_])\\$?${escapeRegExp(coin.symbol)}([^A-Za-z0-9_]|$)`, 'i')
    if (pattern.test(recalled)) used.add(coin.symbol)
  }
  return used
}

export interface PickOptions {
  /** Below this the target is not a moon, it is a neighbour. Defaults to 2. */
  minMultiple?: number
  /** Above this the number stops meaning anything to a reader. Defaults to 2000. */
  maxMultiple?: number
  /** Symbols posted recently; skipped unless nothing else is left. */
  exclude?: Set<string>
  random?: () => number
}

/**
 * Today's target: one coin from the board, above the token, not one that
 * was posted lately, chosen at random within the band where the multiple
 * is still a number a holder can picture. When every candidate has been
 * used the list resets rather than the day being skipped; a repeat a
 * fortnight later is fine, a repeat tomorrow is not, and the exclusions
 * come from memory, which is what makes the difference.
 */
export function pickTarget(token: ListedCoin, board: ListedCoin[], options: PickOptions = {}): ListedCoin | undefined {
  const minMultiple = options.minMultiple ?? 2
  const maxMultiple = options.maxMultiple ?? 2_000
  const random = options.random ?? Math.random
  const above = board.filter((coin) => coin.id !== token.id && coin.marketCapUsd / token.marketCapUsd >= minMultiple)
  if (above.length === 0) return undefined
  const inBand = above.filter((coin) => coin.marketCapUsd / token.marketCapUsd <= maxMultiple)
  const pool = inBand.length > 0 ? inBand : above
  const fresh = pool.filter((coin) => !options.exclude?.has(coin.symbol))
  const candidates = fresh.length > 0 ? fresh : pool
  return candidates[Math.min(candidates.length - 1, Math.floor(random() * candidates.length))]
}

/**
 * The rungs drawn beside the pick: the board spread across the climb from
 * the token's own cap to the biggest coin on the list, so the picture shows
 * where today's target sits among the rest. Picked evenly in log space,
 * since caps on one list span three orders of magnitude, with the target
 * always among them and duplicates folded. Ascending, smallest first. Only
 * coins at least `minMultiple` above the token qualify, as for the pick.
 */
export function ladder(token: ListedCoin, board: ListedCoin[], target: ListedCoin, count = 7, minMultiple = 2): Rung[] {
  // The same floor as the pick: a coin at 1.04x is a neighbour, and a rung
  // that reads "1.04x" next to the token makes the strip look like a typo.
  const above = board
    .filter((coin) => coin.id !== token.id && coin.marketCapUsd / token.marketCapUsd >= minMultiple)
    .sort((a, b) => a.marketCapUsd - b.marketCapUsd)
  if (above.length === 0) return [rung(token, target)]
  const chosen = new Map<string, ListedCoin>()
  chosen.set(target.id, target)
  const slots = Math.max(count - 1, 1)
  const low = Math.log10(above[0]!.marketCapUsd)
  const high = Math.log10(above[above.length - 1]!.marketCapUsd)
  for (let slot = 0; slot < slots && chosen.size < count; slot += 1) {
    const wanted = slots === 1 ? high : low + ((high - low) * slot) / (slots - 1)
    // The coin nearest the wanted cap that is not already on the ladder.
    const nearest = above
      .filter((coin) => !chosen.has(coin.id))
      .sort((a, b) => Math.abs(Math.log10(a.marketCapUsd) - wanted) - Math.abs(Math.log10(b.marketCapUsd) - wanted))[0]
    if (nearest) chosen.set(nearest.id, nearest)
  }
  return [...chosen.values()].sort((a, b) => a.marketCapUsd - b.marketCapUsd).map((coin) => rung(token, coin))
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}
