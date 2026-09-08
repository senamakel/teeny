import { describe, expect, it } from 'vitest'
import { analyse } from '../src/ta/analysis.js'
import type { Candle } from '../src/market/types.js'

function candles(closes: number[], volumes?: number[]): Candle[] {
  return closes.map((close, index) => {
    const open = index === 0 ? close : (closes[index - 1] as number)
    return {
      time: 1_700_000_000 + index * 3_600,
      open,
      high: Math.max(open, close) * 1.005,
      low: Math.min(open, close) * 0.995,
      close,
      volume: volumes?.[index] ?? 100,
    }
  })
}

const rising = Array.from({ length: 120 }, (_, index) => 100 + index * 1.5)
const falling = Array.from({ length: 120 }, (_, index) => 300 - index * 1.5)

describe('technical read', () => {
  it('refuses to analyse a series it cannot say anything about', () => {
    expect(() => analyse(candles([1, 2]), { timeframe: '1h' })).toThrow(/at least 3/)
  })

  it('reads a clean uptrend as bullish', () => {
    const read = analyse(candles(rising), { timeframe: '1h' })
    expect(read.trend).toBe('up')
    expect(read.changePct).toBeGreaterThan(0)
    expect(read.bullishScore).toBeGreaterThan(60)
  })

  /**
   * The regression that mattered. Without the window-trend signal, a token down
   * a third over five days scored bullish on a two-hour bounce, and a bullish
   * account would happily have posted that under a chart that was visibly
   * bleeding.
   */
  it('does not score a bounce inside a downtrend as bullish', () => {
    const bounce = [...falling.slice(0, 112), 140, 143, 147, 152, 158, 165, 173, 182]
    const read = analyse(candles(bounce), { timeframe: '1h' })

    expect(read.changePct).toBeLessThan(0)
    expect(read.signals.some((signal) => signal.name === 'window trend' && !signal.bullish)).toBe(true)
    expect(read.bullishScore).toBeLessThan(75)
  })

  it('scores a sustained downtrend as weak', () => {
    const read = analyse(candles(falling), { timeframe: '1h' })
    expect(read.trend).toBe('down')
    expect(read.bullishScore).toBeLessThan(40)
  })

  it('reports levels only where they exist and keeps them on the right side of price', () => {
    const read = analyse(candles(rising), { timeframe: '1h' })
    if (read.levels.support !== undefined) expect(read.levels.support).toBeLessThan(read.price)
    if (read.levels.resistance !== undefined) expect(read.levels.resistance).toBeGreaterThan(read.price)
    expect(read.levels.windowHigh).toBeGreaterThanOrEqual(read.levels.windowLow)
  })

  it('notices a volume spike on the last bar', () => {
    const volumes = Array.from({ length: 120 }, (_, index) => (index === 119 ? 5_000 : 100))
    const read = analyse(candles(rising, volumes), { timeframe: '1h' })
    expect(read.signals.some((signal) => signal.name === 'volume')).toBe(true)
    expect(read.volume.ratio).toBeGreaterThan(1.5)
  })

  it('writes notes that quote only numbers it computed', () => {
    const read = analyse(candles(rising), { timeframe: '1h' })
    expect(read.notes.join(' ')).toContain(`${read.bullishScore}/100`)
    expect(read.notes.some((note) => note.includes('bars'))).toBe(true)
  })
})
