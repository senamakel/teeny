import { describe, expect, it } from 'vitest'
import { atr, bollinger, ema, latest, levels, sma, swings, vwap } from '../src/ta/indicators.js'
import type { Candle } from '../src/market/types.js'

function series(closes: number[]): Candle[] {
  return closes.map((close, index) => ({
    time: 1_700_000_000 + index * 3_600,
    open: index === 0 ? close : (closes[index - 1] as number),
    high: close * 1.01,
    low: close * 0.99,
    close,
    volume: 100,
  }))
}

describe('moving averages', () => {
  it('has no value until it has enough history', () => {
    const values = sma([1, 2, 3, 4, 5], 3)
    expect(values.slice(0, 2)).toEqual([undefined, undefined])
    expect(values[2]).toBe(2)
    expect(values[4]).toBe(4)
  })

  it('seeds the EMA on the simple average, not on the first close', () => {
    // Seeded on the first close, the first defined EMA(3) here would be 1;
    // seeded on the SMA it is 2, which is where a charting package puts it.
    expect(ema([1, 2, 3, 4, 5], 3)[2]).toBe(2)
  })

  it('returns nothing at all when the series is shorter than the period', () => {
    expect(ema([1, 2], 5).every((value) => value === undefined)).toBe(true)
  })
})

describe('bands and range', () => {
  it('brackets the price and widens with volatility', () => {
    const calm = bollinger(Array.from({ length: 40 }, () => 100), 20)
    expect(latest(calm.upper)).toBeCloseTo(100, 6)
    expect(latest(calm.lower)).toBeCloseTo(100, 6)

    const choppy = bollinger(
      Array.from({ length: 40 }, (_, index) => (index % 2 === 0 ? 90 : 110)),
      20,
    )
    expect(latest(choppy.upper)!).toBeGreaterThan(latest(calm.upper)!)
  })

  it('measures true range against the previous close', () => {
    const candles = series([10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25])
    expect(latest(atr(candles, 14))!).toBeGreaterThan(0)
  })

  it('weights VWAP by volume', () => {
    const candles = series([10, 20])
    candles[0]!.volume = 999
    candles[1]!.volume = 1
    expect(vwap(candles)!).toBeLessThan(11)
  })
})

describe('swings and levels', () => {
  it('will not call a swing on the last bars, because nothing has confirmed them', () => {
    const candles = series([1, 2, 3, 2, 1, 2, 3, 4, 5])
    const found = swings(candles, 2)
    expect(found.every((point) => point.index < candles.length - 2)).toBe(true)
  })

  it('clusters repeated touches into one level', () => {
    // Three visits to roughly the same high, and one much lower low.
    const candles = series([10, 12, 10, 12.05, 10, 11.98, 10, 12.02, 10, 12, 10])
    const found = levels(candles, { tolerance: 0.02 })
    expect(found[0]!.touches).toBeGreaterThan(1)
  })
})
