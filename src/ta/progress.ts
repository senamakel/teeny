import type { Candle } from '../market/types.js'
import { swings } from './indicators.js'

export interface HorizonChange {
  /** How many days back, e.g. 7. */
  days: number
  changePct: number
}

export interface Progress {
  days: number
  price: number
  changes: HorizonChange[]
  /** Highest and lowest close in the window, and how far price is from each. */
  high: number
  low: number
  fromHighPct: number
  fromLowPct: number
  /** Days closed green out of the days read. */
  greenDays: number
  /** Whether the daily lows are stepping up, which is the long-horizon version of structure. */
  higherLows: boolean
  /** True, positive things about the long horizon, strongest first. Never empty. */
  positives: string[]
  notes: string[]
}

/**
 * The long horizon, which is the part a holder actually lives in.
 *
 * The hourly read answers "what happened today". This answers "are we further
 * along than we were", which is a different question and the one somebody
 * holding through a drawdown is really asking. It is deliberately coarse:
 * changes over weeks, how many days closed green, whether the lows are
 * stepping up. No indicator here at all -- on a daily memecoin series there is
 * not enough history for one to mean anything, and the honest long-term
 * signals are arithmetic.
 */
export function readProgress(daily: Candle[]): Progress {
  if (daily.length < 2) throw new Error('Not enough daily candles to read progress: need at least 2')

  const last = daily[daily.length - 1] as Candle
  const closesOnly = daily.map((candle) => candle.close)
  const high = Math.max(...closesOnly)
  const low = Math.min(...closesOnly)

  const changes = [7, 30, 90]
    .filter((days) => daily.length > days)
    .map((days) => {
      const from = daily[daily.length - 1 - days] as Candle
      return { days, changePct: round(pct(from.close, last.close)) }
    })
  // Whatever history there is, if it is shorter than the shortest horizon
  // above. A token that launched three weeks ago still has a story.
  if (changes.length === 0) {
    const first = daily[0] as Candle
    changes.push({ days: daily.length - 1, changePct: round(pct(first.close, last.close)) })
  }

  const lows = swings(daily, 2)
    .filter((point) => point.kind === 'low')
    .slice(-3)
    .map((point) => point.price)

  const progress: Progress = {
    days: daily.length,
    price: last.close,
    changes,
    high,
    low,
    fromHighPct: round(pct(high, last.close)),
    fromLowPct: round(pct(low, last.close)),
    greenDays: daily.filter((candle) => candle.close >= candle.open).length,
    higherLows: lows.length >= 2 && lows.every((value, index) => index === 0 || value > (lows[index - 1] as number)),
    positives: [],
    notes: [],
  }

  // The long horizon is where the positive usually lives on a red day: an hour
  // can be ugly while the month is not, and both are true at once.
  const gains = changes.filter((change) => change.changePct > 0)
  if (gains.length > 0) {
    progress.positives.push(
      ...gains.map((change) => `up ${change.changePct}% over ${change.days} days`),
    )
  }
  if (progress.higherLows) progress.positives.push('the daily lows are still stepping up')
  if (progress.greenDays > progress.days / 2) {
    progress.positives.push(`${progress.greenDays} of the last ${progress.days} days closed green`)
  }
  progress.positives.push(`price is ${progress.fromLowPct}% above the low of this stretch`)

  progress.notes = [
    `Over ${progress.days} days: ` +
      changes.map((change) => `${change.days}d ${change.changePct >= 0 ? '+' : ''}${change.changePct}%`).join(', ') +
      '.',
    `${progress.greenDays} of ${progress.days} days closed green.`,
    `Daily high $${high.toPrecision(4)}, low $${low.toPrecision(4)}; price is ` +
      `${Math.abs(progress.fromHighPct)}% off the high and ${progress.fromLowPct}% off the low.`,
    progress.higherLows
      ? 'On the daily, the lows are stepping up.'
      : 'On the daily, the lows are not yet stepping up.',
    `TRUE POSITIVES YOU CAN LEAD WITH, best first: ${progress.positives.join('; ')}.`,
  ]
  return progress
}

function pct(from: number, to: number): number {
  return from === 0 ? 0 : ((to - from) / from) * 100
}

function round(value: number, places = 2): number {
  const factor = 10 ** places
  return Math.round(value * factor) / factor
}
