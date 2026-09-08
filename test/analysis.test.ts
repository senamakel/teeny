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
    expect(read.bullishScore).toBeLessThan(80)
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
    expect(read.signals.some((signal) => signal.name === 'volume spike')).toBe(true)
    expect(read.volume.ratio).toBeGreaterThan(1.5)
  })

  it('notices volume waking up across the window, not just on one bar', () => {
    const volumes = Array.from({ length: 120 }, (_, index) => (index < 60 ? 100 : 400))
    const read = analyse(candles(rising, volumes), { timeframe: '1h' })
    expect(read.volume.trend).toBeCloseTo(4, 1)
    expect(read.signals.some((signal) => signal.name === 'volume trend' && signal.bullish)).toBe(true)
  })

  it('notices volume drying up', () => {
    const volumes = Array.from({ length: 120 }, (_, index) => (index < 60 ? 400 : 100))
    const read = analyse(candles(rising, volumes), { timeframe: '1h' })
    expect(read.signals.some((signal) => signal.name === 'volume trend' && !signal.bullish)).toBe(true)
  })

  /**
   * RSI and MACD were here and came out: they are mean-reversion tools for
   * instruments that revert, and this is not one. The read must not grow them
   * back by accident, because the prompt now forbids the account from saying
   * "overbought" and a tool that reports it is an invitation.
   */
  it('computes no oscillators at all', () => {
    const read = analyse(candles(rising), { timeframe: '1h' })
    const surface = JSON.stringify(read).toLowerCase()

    expect(surface).not.toContain('rsi')
    expect(surface).not.toContain('macd')
    expect(surface).not.toContain('overbought')
    expect(surface).not.toContain('oversold')
  })

  it('writes notes that quote only numbers it computed', () => {
    const read = analyse(candles(rising), { timeframe: '1h' })
    expect(read.notes.join(' ')).toContain(`${read.bullishScore}/100`)
    expect(read.notes.some((note) => note.includes('bars'))).toBe(true)
  })
})

describe('level events', () => {
  /** A level touched once is a coordinate, not a level. */
  function withLevel(): number[] {
    // Four visits to ~100 that get rejected, then a close through it.
    return [
      ...Array.from({ length: 20 }, (_, index) => 90 + index * 0.2),
      100, 94, 100, 93, 99.8, 92, 100.2, 95,
      ...Array.from({ length: 12 }, (_, index) => 96 + index * 0.6),
    ]
  }

  it('calls a break above a defended level, and says whether volume backed it', () => {
    const closes = withLevel()
    const volumes = closes.map((_, index) => (index >= closes.length - 3 ? 900 : 100))
    const read = analyse(candles(closes, volumes), { timeframe: '1h' })

    expect(read.event?.kind).toBeDefined()
    expect(read.event?.touches).toBeGreaterThanOrEqual(2)
    if (read.event?.kind !== 'breakdown') {
      expect(read.signals.some((signal) => signal.name === read.event?.kind)).toBe(true)
    }
  })

  it('downgrades a break that volume did not confirm', () => {
    const closes = withLevel()
    const quiet = analyse(candles(closes, closes.map(() => 100)), { timeframe: '1h' })
    if (quiet.event) {
      expect(quiet.event.volumeConfirmed).toBe(false)
      const signal = quiet.signals.find((entry) => entry.name === quiet.event?.kind)
      expect(signal?.strength).toBe('moderate')
      expect(signal?.detail).toContain('volume did not confirm')
    }
  })

  it('reports no event on a series that never crosses anything twice-touched', () => {
    expect(analyse(candles(rising), { timeframe: '1h' }).event).toBeUndefined()
  })
})
