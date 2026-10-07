import { describe, expect, it } from 'vitest'
import { fold, TIMEFRAMES, type Candle } from '../src/market/types.js'

const DAY = 86_400

function daily(count: number, from = 0): Candle[] {
  return Array.from({ length: count }, (_, index) => ({
    // A Thursday-aligned epoch, so bucket boundaries are not accidentally tidy.
    time: from + index * DAY,
    open: 10 + index,
    high: 12 + index,
    low: 8 + index,
    close: 11 + index,
    volume: 100,
  }))
}

describe('timeframes', () => {
  it('knows weekly has to be built rather than fetched', () => {
    // GeckoTerminal answers 400 to any daily aggregate but 1, so a weekly
    // candle does not exist upstream and has to be folded here.
    expect(TIMEFRAMES['1w'].foldFrom).toEqual({ timeframe: '1d', factor: 7 })
    expect(TIMEFRAMES['1d'].foldFrom).toBeUndefined()
    expect(TIMEFRAMES['12h'].seconds).toBe(43_200)
  })
})

describe('folding', () => {
  it('takes the first open, the last close, and the extremes of the bucket', () => {
    const week = fold(daily(7), DAY * 7)
    expect(week).toHaveLength(1)
    expect(week[0]).toMatchObject({
      open: 10,
      close: 17,
      high: 18,
      low: 8,
      volume: 700,
    })
  })

  it('sums volume rather than averaging it', () => {
    expect(fold(daily(14), DAY * 7).map((candle) => candle.volume)).toEqual([700, 700])
  })

  /**
   * Bucketed on absolute time, not by counting back from the newest bar.
   * Counting back would silently redraw every weekly candle depending on what
   * hour the daily job happened to run.
   */
  it('puts a bar in the same bucket regardless of what else was fetched', () => {
    const all = fold(daily(21), DAY * 7)
    const fewer = fold(daily(21).slice(3), DAY * 7)
    const shared = all.filter((candle) => fewer.some((other) => other.time === candle.time))

    expect(shared.length).toBeGreaterThan(0)
    for (const candle of shared.slice(1)) {
      expect(fewer.find((other) => other.time === candle.time)).toEqual(candle)
    }
  })

  it('keeps a partial bucket rather than dropping the current week', () => {
    // Ten days is one full week and three days of the next one.
    const weeks = fold(daily(10), DAY * 7)
    expect(weeks).toHaveLength(2)
    expect(weeks[1]!.volume).toBe(300)
  })

  it('returns nothing for nothing', () => {
    expect(fold([], DAY * 7)).toEqual([])
  })
})
