import type { Candle } from '../market/types.js'

/**
 * The indicator kit.
 *
 * Every function returns an array the same length as its input, with
 * `undefined` for the bars before the indicator has enough history. That is the
 * point: an EMA(50) over 40 bars is not zero and it is not the first close, it
 * is unknown, and a chart that draws it anyway invents a trend line out of the
 * warm-up period. Callers read the last defined value and say nothing when
 * there is not one.
 */

export type Series = Array<number | undefined>

export function closes(candles: Candle[]): number[] {
  return candles.map((candle) => candle.close)
}

export function sma(values: number[], period: number): Series {
  if (period <= 0) throw new Error('period must be positive')
  const out: Series = []
  let total = 0
  for (let index = 0; index < values.length; index += 1) {
    total += values[index] as number
    if (index >= period) total -= values[index - period] as number
    out.push(index >= period - 1 ? total / period : undefined)
  }
  return out
}

/**
 * Seeded with the SMA of the first `period` values rather than with the first
 * value, which is what every charting package does. Seeding on one close makes
 * the early curve a decay from a single arbitrary point.
 */
export function ema(values: number[], period: number): Series {
  if (period <= 0) throw new Error('period must be positive')
  const out: Series = Array.from<number | undefined>({ length: values.length }).fill(undefined)
  if (values.length < period) return out

  const k = 2 / (period + 1)
  let previous = values.slice(0, period).reduce((total, value) => total + value, 0) / period
  out[period - 1] = previous
  for (let index = period; index < values.length; index += 1) {
    previous = (values[index] as number) * k + previous * (1 - k)
    out[index] = previous
  }
  return out
}

/** Wilder's RSI: smoothed averages, not simple ones. */
export function rsi(values: number[], period = 14): Series {
  const out: Series = Array.from<number | undefined>({ length: values.length }).fill(undefined)
  if (values.length <= period) return out

  let gains = 0
  let losses = 0
  for (let index = 1; index <= period; index += 1) {
    const change = (values[index] as number) - (values[index - 1] as number)
    if (change >= 0) gains += change
    else losses -= change
  }
  gains /= period
  losses /= period
  out[period] = losses === 0 ? 100 : 100 - 100 / (1 + gains / losses)

  for (let index = period + 1; index < values.length; index += 1) {
    const change = (values[index] as number) - (values[index - 1] as number)
    gains = (gains * (period - 1) + Math.max(change, 0)) / period
    losses = (losses * (period - 1) + Math.max(-change, 0)) / period
    out[index] = losses === 0 ? 100 : 100 - 100 / (1 + gains / losses)
  }
  return out
}

export interface Macd {
  macd: Series
  signal: Series
  histogram: Series
}

export function macd(values: number[], fast = 12, slow = 26, signalPeriod = 9): Macd {
  const fastLine = ema(values, fast)
  const slowLine = ema(values, slow)
  const line: Series = values.map((_, index) => {
    const a = fastLine[index]
    const b = slowLine[index]
    return a === undefined || b === undefined ? undefined : a - b
  })

  // The signal is an EMA of the MACD line, so it can only start where the line
  // does; feeding the undefined head in as zeroes drags it below zero for the
  // whole warm-up and prints a crossover that never happened.
  const defined = line.flatMap((value) => (value === undefined ? [] : [value]))
  const offset = line.findIndex((value) => value !== undefined)
  const signalDefined = offset < 0 ? [] : ema(defined, signalPeriod)
  const signal: Series = Array.from<number | undefined>({ length: values.length }).fill(undefined)
  signalDefined.forEach((value, index) => {
    signal[offset + index] = value
  })

  return {
    macd: line,
    signal,
    histogram: line.map((value, index) => {
      const reference = signal[index]
      return value === undefined || reference === undefined ? undefined : value - reference
    }),
  }
}

export interface Bands {
  upper: Series
  middle: Series
  lower: Series
}

export function bollinger(values: number[], period = 20, deviations = 2): Bands {
  const middle = sma(values, period)
  const upper: Series = []
  const lower: Series = []
  for (let index = 0; index < values.length; index += 1) {
    const mean = middle[index]
    if (mean === undefined) {
      upper.push(undefined)
      lower.push(undefined)
      continue
    }
    const window = values.slice(index - period + 1, index + 1)
    const variance =
      window.reduce((total, value) => total + (value - mean) ** 2, 0) / window.length
    const sigma = Math.sqrt(variance)
    upper.push(mean + sigma * deviations)
    lower.push(mean - sigma * deviations)
  }
  return { upper, middle, lower }
}

/** Average true range, Wilder-smoothed. Used here as a volatility scale, not a stop. */
export function atr(candles: Candle[], period = 14): Series {
  const out: Series = Array.from<number | undefined>({ length: candles.length }).fill(undefined)
  if (candles.length <= period) return out

  const ranges = candles.map((candle, index) => {
    const previous = candles[index - 1]
    if (!previous) return candle.high - candle.low
    return Math.max(
      candle.high - candle.low,
      Math.abs(candle.high - previous.close),
      Math.abs(candle.low - previous.close),
    )
  })

  let average = ranges.slice(1, period + 1).reduce((total, value) => total + value, 0) / period
  out[period] = average
  for (let index = period + 1; index < candles.length; index += 1) {
    average = (average * (period - 1) + (ranges[index] as number)) / period
    out[index] = average
  }
  return out
}

/** Volume-weighted average price over the whole series. */
export function vwap(candles: Candle[]): number | undefined {
  const volume = candles.reduce((total, candle) => total + candle.volume, 0)
  if (volume <= 0) return undefined
  const weighted = candles.reduce(
    (total, candle) => total + ((candle.high + candle.low + candle.close) / 3) * candle.volume,
    0,
  )
  return weighted / volume
}

export interface Swing {
  index: number
  time: number
  price: number
  kind: 'high' | 'low'
}

/**
 * Fractal swing points: a bar higher (or lower) than `span` bars either side.
 *
 * The last `span` bars can never qualify, because the bars that would confirm
 * them have not printed yet. That is correct and deliberate -- a "swing high"
 * called on the final bar is just the current price with a name on it.
 */
export function swings(candles: Candle[], span = 2): Swing[] {
  const found: Swing[] = []
  for (let index = span; index < candles.length - span; index += 1) {
    const candle = candles[index]
    if (!candle) continue
    const window = candles.slice(index - span, index + span + 1)
    if (window.every((other) => other.high <= candle.high)) {
      found.push({ index, time: candle.time, price: candle.high, kind: 'high' })
    }
    if (window.every((other) => other.low >= candle.low)) {
      found.push({ index, time: candle.time, price: candle.low, kind: 'low' })
    }
  }
  return found
}

/**
 * Price levels that have been touched more than once, strongest first.
 *
 * Swing points are clustered by relative distance rather than absolute, so this
 * behaves the same on a token priced at 0.0007 and one priced at 40,000.
 */
export function levels(candles: Candle[], options: { span?: number; tolerance?: number } = {}): Array<{ price: number; touches: number }> {
  const tolerance = options.tolerance ?? 0.015
  const points = swings(candles, options.span ?? 2)
  const clusters: Array<{ price: number; touches: number }> = []

  for (const point of points.sort((a, b) => a.price - b.price)) {
    const last = clusters[clusters.length - 1]
    if (last && Math.abs(point.price - last.price) / last.price <= tolerance) {
      // Running mean, so a cluster's price is where the touches actually were
      // rather than wherever the first one happened to land.
      last.price = (last.price * last.touches + point.price) / (last.touches + 1)
      last.touches += 1
    } else {
      clusters.push({ price: point.price, touches: 1 })
    }
  }
  return clusters.sort((a, b) => b.touches - a.touches)
}

/** The last defined value of a series, which is the only one a post ever quotes. */
export function latest(series: Series): number | undefined {
  for (let index = series.length - 1; index >= 0; index -= 1) {
    const value = series[index]
    if (value !== undefined) return value
  }
  return undefined
}
