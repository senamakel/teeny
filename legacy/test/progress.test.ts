import { describe, expect, it } from 'vitest'
import { readProgress } from '../src/ta/progress.js'
import type { Candle } from '../src/market/types.js'

function daily(closes: number[]): Candle[] {
  return closes.map((close, index) => {
    const open = index === 0 ? close : (closes[index - 1] as number)
    return {
      time: 1_700_000_000 + index * 86_400,
      open,
      high: Math.max(open, close) * 1.02,
      low: Math.min(open, close) * 0.98,
      close,
      volume: 1_000,
    }
  })
}

describe('long-horizon progress', () => {
  it('measures the horizons it has the history for', () => {
    const progress = readProgress(daily(Array.from({ length: 100 }, (_, index) => 1 + index * 0.1)))
    expect(progress.changes.map((change) => change.days)).toEqual([7, 30, 90])
    expect(progress.changes[0]!.changePct).toBeGreaterThan(0)
  })

  /** A token three weeks old still has a story; it just is not a 90-day one. */
  it('falls back to whatever history exists when the token is young', () => {
    const progress = readProgress(daily([1, 1.2, 1.4, 1.8]))
    expect(progress.changes).toEqual([{ days: 3, changePct: 80 }])
  })

  it('counts green days and finds stepping lows', () => {
    // Each pullback bottoms higher than the last: 14, then 18, then 24.
    const progress = readProgress(daily([20, 14, 20, 26, 18, 26, 32, 24, 32, 38, 30, 38]))
    expect(progress.greenDays).toBeGreaterThan(0)
    expect(progress.higherLows).toBe(true)
  })

  it('says plainly when the lows are not stepping up', () => {
    const progress = readProgress(daily([20, 18, 19, 15, 16, 12, 13, 9, 10]))
    expect(progress.higherLows).toBe(false)
    expect(progress.notes.join(' ')).toContain('not yet stepping up')
  })

  it('finds the positive on the long horizon, which is where it lives on a red day', () => {
    const progress = readProgress(daily([1, 2, 4, 8, 16, 32, 30, 28]))
    expect(progress.positives.length).toBeGreaterThan(0)
    expect(progress.notes.join(' ')).toContain('TRUE POSITIVES YOU CAN LEAD WITH')
  })

  it('still has one when every horizon is down', () => {
    const progress = readProgress(daily(Array.from({ length: 40 }, (_, index) => 100 - index * 2)))
    expect(progress.positives.length).toBeGreaterThan(0)
    expect(progress.positives.every((entry) => !entry.includes('up 0'))).toBe(true)
  })

  it('refuses to read progress from a single day', () => {
    expect(() => readProgress(daily([1]))).toThrow(/at least 2/)
  })

  it('places price against its own daily range', () => {
    const progress = readProgress(daily([10, 20, 15]))
    expect(progress.high).toBe(20)
    expect(progress.low).toBe(10)
    expect(progress.fromHighPct).toBeLessThan(0)
    expect(progress.fromLowPct).toBeGreaterThan(0)
  })
})
