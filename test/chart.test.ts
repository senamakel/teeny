import { describe, expect, it } from 'vitest'
import { compactUsd, escapeXml, price } from '../src/chart/format.js'
import { percentile, priceTicks, renderChartSvg } from '../src/chart/svg.js'
import type { Candle } from '../src/market/types.js'

function candles(count: number): Candle[] {
  return Array.from({ length: count }, (_, index) => ({
    time: 1_700_000_000 + index * 3_600,
    open: 0.0007 + index * 0.000001,
    high: 0.00071 + index * 0.000001,
    low: 0.00069 + index * 0.000001,
    close: 0.000705 + index * 0.000001,
    volume: 100,
  }))
}

describe('formatting', () => {
  it('keeps four significant figures on a sub-cent price', () => {
    expect(price(0.0007472834089)).toBe('0.0007473')
    expect(price(0)).toBe('0')
  })

  it('compacts money the way a chart header does', () => {
    expect(compactUsd(730_709)).toBe('$730.7K')
    expect(compactUsd(1_240_000)).toBe('$1.24M')
    expect(compactUsd(999)).toBe('$999')
    expect(compactUsd(93_428)).toBe('$93.4K')
  })

  it('escapes the characters a token name can carry into markup', () => {
    expect(escapeXml('<a & "b">')).toBe('&lt;a &amp; &quot;b&quot;&gt;')
  })
})

describe('axis', () => {
  it('picks round steps inside the range', () => {
    const ticks = priceTicks(0, 10, 5)
    expect(ticks[0]).toBe(0)
    expect(ticks[ticks.length - 1]).toBeLessThanOrEqual(10)
    expect(ticks.length).toBeGreaterThan(2)
  })

  it('survives a flat series without dividing by zero', () => {
    expect(priceTicks(5, 5, 6)).toEqual([5])
  })
})

describe('volume scaling', () => {
  it('takes the 95th percentile, so one launch bar cannot flatten the panel', () => {
    const volumes = [...Array.from({ length: 99 }, () => 10), 100_000]
    expect(percentile(volumes, 0.95)).toBe(10)
  })
})

describe('chart svg', () => {
  it('draws a body and a wick for every candle', () => {
    const svg = renderChartSvg({ candles: candles(30), symbol: '$TINY', timeframe: '1h' })
    // One wick line per candle, plus grid lines and the last-price rule.
    expect((svg.match(/<rect /g) ?? []).length).toBeGreaterThanOrEqual(60)
    expect(svg.startsWith('<svg')).toBe(true)
    expect(svg.trimEnd().endsWith('</svg>')).toBe(true)
  })

  it('refuses to draw nothing', () => {
    expect(() => renderChartSvg({ candles: [], symbol: '$TINY', timeframe: '1h' })).toThrow(
      /no candles/,
    )
  })

  it('breaks an overlay line where the indicator has no value', () => {
    const svg = renderChartSvg({
      candles: candles(10),
      symbol: '$TINY',
      timeframe: '1h',
      overlays: [
        {
          label: 'EMA5',
          color: '#fff',
          values: [undefined, undefined, 0.0007, 0.00071, undefined, 0.00072, 0.00073, 0.00074, 0.00075, 0.00076],
        },
      ],
    })
    const path = svg.match(/<path d="([^"]+)"/)?.[1] ?? ''
    // Two move commands: the gap starts a new segment rather than drawing
    // straight across it.
    expect((path.match(/M/g) ?? []).length).toBe(2)
  })

  it('puts the header, the change and the stats where a reader looks', () => {
    const svg = renderChartSvg({
      candles: candles(20),
      symbol: '$TINY',
      name: 'Tiny Humans AI',
      timeframe: '1h',
      changePct: 17.4,
      stats: [{ label: 'MC', value: '$730.7K' }],
    })
    expect(svg).toContain('$TINY')
    expect(svg).toContain('+17.40%')
    expect(svg).toContain('MC $730.7K')
  })

  it('keeps an overlay that runs past the candles inside the panel', () => {
    const svg = renderChartSvg({
      candles: candles(20),
      symbol: '$TINY',
      timeframe: '1h',
      // Far above every high: the scale has to grow to include it.
      overlays: [{ label: 'x', color: '#fff', values: candles(20).map(() => 0.002) }],
    })
    const ys = [...svg.matchAll(/<path d="M[\d.]+ ([\d.]+)/g)].map((match) => Number(match[1]))
    expect(ys.every((y) => y > 0)).toBe(true)
  })
})
