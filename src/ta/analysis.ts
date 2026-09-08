import type { Candle, Timeframe } from '../market/types.js'
import { atr, bollinger, closes, ema, latest, levels, macd, rsi, sma, swings, vwap } from './indicators.js'

export type Strength = 'weak' | 'moderate' | 'strong'

export interface Signal {
  name: string
  /** Which way it reads. A bearish signal is still reported: see `analyse`. */
  bullish: boolean
  strength: Strength
  detail: string
}

export interface TechnicalRead {
  timeframe: Timeframe
  bars: number
  price: number
  /** Change over the whole window read, in percent. */
  changePct: number
  trend: 'up' | 'down' | 'sideways'
  structure: string
  ema: { fast?: number; slow?: number; fastPeriod: number; slowPeriod: number }
  rsi?: number
  macd: { line?: number; signal?: number; histogram?: number }
  bollinger: { upper?: number; lower?: number; widthPct?: number; positionPct?: number }
  atrPct?: number
  vwap?: number
  volume: { last: number; average: number; ratio: number }
  levels: { support?: number; resistance?: number; windowHigh: number; windowLow: number }
  fromHighPct: number
  fromLowPct: number
  signals: Signal[]
  /**
   * 0-100, where 50 is neutral. Not a prediction and not a recommendation: it
   * is the balance of the signals below, and it exists so a post can be honest
   * about a weak tape instead of finding something nice to say about it.
   */
  bullishScore: number
  /** Plain sentences a post can be built from, each one true of the data above. */
  notes: string[]
}

const WEIGHT: Record<Strength, number> = { weak: 1, moderate: 2, strong: 3 }

function pct(from: number, to: number): number {
  return from === 0 ? 0 : ((to - from) / from) * 100
}

function round(value: number, places = 2): number {
  const factor = 10 ** places
  return Math.round(value * factor) / factor
}

/**
 * Everything the agent is allowed to say about the tape, computed rather than
 * imagined.
 *
 * This deliberately reports bearish signals too, even though the account it
 * feeds is unapologetically long. A bullish poster that cannot see a downtrend
 * eventually posts "accumulation zone" into a chart that is bleeding, and the
 * timeline notices. The tone lives in the prompt; the truth lives here, and
 * `bullishScore` is what lets the prompt say "if this is low, post about
 * something other than the price today".
 */
export function analyse(
  candles: Candle[],
  options: { timeframe: Timeframe; fastPeriod?: number; slowPeriod?: number } = {
    timeframe: '1h',
  },
): TechnicalRead {
  if (candles.length < 3) throw new Error('Not enough candles to analyse: need at least 3')

  const fastPeriod = options.fastPeriod ?? 20
  const slowPeriod = options.slowPeriod ?? 50
  const values = closes(candles)
  const last = candles[candles.length - 1] as Candle
  const first = candles[0] as Candle

  const fastEma = ema(values, fastPeriod)
  const slowEma = ema(values, slowPeriod)
  const fast = latest(fastEma)
  const slow = latest(slowEma)
  const strength = rsi(values, 14)
  const momentum = macd(values)
  const bands = bollinger(values, 20, 2)
  const range = atr(candles, 14)
  const volumeAverage = latest(sma(candles.map((candle) => candle.volume), Math.min(20, candles.length))) ?? 0

  const highs = candles.map((candle) => candle.high)
  const lows = candles.map((candle) => candle.low)
  const windowHigh = Math.max(...highs)
  const windowLow = Math.min(...lows)

  const upper = latest(bands.upper)
  const lower = latest(bands.lower)
  const middle = latest(bands.middle)
  const rsiValue = latest(strength)
  const histogram = latest(momentum.histogram)
  const atrValue = latest(range)

  const points = swings(candles, 2)
  const swingHighs = points.filter((point) => point.kind === 'high').slice(-3)
  const swingLows = points.filter((point) => point.kind === 'low').slice(-3)
  const higherHighs = rising(swingHighs.map((point) => point.price))
  const higherLows = rising(swingLows.map((point) => point.price))
  const lowerHighs = falling(swingHighs.map((point) => point.price))
  const lowerLows = falling(swingLows.map((point) => point.price))

  const found = levels(candles)
  const support = found
    .filter((level) => level.price < last.close)
    .sort((a, b) => b.price - a.price)[0]
  const resistance = found
    .filter((level) => level.price > last.close)
    .sort((a, b) => a.price - b.price)[0]

  const windowChange = pct(first.open, last.close)

  const signals: Signal[] = []
  const add = (signal: Signal): void => {
    signals.push(signal)
  }

  // The whole window, weighted heavily on purpose. Without it a token down 50%
  // over five days scores bullish on a two-hour bounce, and the account posts
  // "structure intact" under a chart that is visibly bleeding.
  add({
    name: 'window trend',
    bullish: windowChange >= 0,
    strength: Math.abs(windowChange) > 15 ? 'strong' : Math.abs(windowChange) > 5 ? 'moderate' : 'weak',
    detail: `${windowChange >= 0 ? 'up' : 'down'} ${round(Math.abs(windowChange))}% across the ${candles.length} bars read`,
  })

  if (fast !== undefined && slow !== undefined) {
    const above = fast > slow
    add({
      name: `EMA${fastPeriod}/EMA${slowPeriod}`,
      bullish: above,
      strength: Math.abs(pct(slow, fast)) > 2 ? 'strong' : 'moderate',
      detail: above
        ? `the ${fastPeriod} EMA is ${round(pct(slow, fast))}% above the ${slowPeriod}`
        : `the ${fastPeriod} EMA is ${round(Math.abs(pct(slow, fast)))}% below the ${slowPeriod}`,
    })
  }
  if (fast !== undefined) {
    const above = last.close > fast
    add({
      name: 'price vs fast EMA',
      bullish: above,
      strength: 'weak',
      detail: `price is ${above ? 'holding above' : 'under'} the ${fastPeriod} EMA`,
    })
  }
  if (rsiValue !== undefined) {
    add({
      name: 'RSI(14)',
      bullish: rsiValue >= 50,
      strength: rsiValue >= 70 || rsiValue <= 30 ? 'strong' : rsiValue >= 55 || rsiValue <= 45 ? 'moderate' : 'weak',
      detail:
        rsiValue >= 70
          ? `RSI ${round(rsiValue, 1)}, overbought and running hot`
          : rsiValue <= 30
            ? `RSI ${round(rsiValue, 1)}, oversold`
            : `RSI ${round(rsiValue, 1)}`,
    })
  }
  if (histogram !== undefined) {
    add({
      name: 'MACD',
      bullish: histogram > 0,
      strength: 'moderate',
      detail: histogram > 0 ? 'MACD histogram is positive' : 'MACD histogram is negative',
    })
  }
  if (higherHighs && higherLows) {
    add({ name: 'structure', bullish: true, strength: 'strong', detail: 'higher highs and higher lows' })
  } else if (lowerHighs && lowerLows) {
    add({ name: 'structure', bullish: false, strength: 'strong', detail: 'lower highs and lower lows' })
  } else if (higherLows) {
    add({ name: 'structure', bullish: true, strength: 'moderate', detail: 'higher lows into resistance' })
  }
  if (volumeAverage > 0) {
    const ratio = last.volume / volumeAverage
    if (ratio >= 1.5) {
      add({
        name: 'volume',
        bullish: last.close >= last.open,
        strength: ratio >= 3 ? 'strong' : 'moderate',
        detail: `last bar traded ${round(ratio, 1)}x the average volume`,
      })
    }
  }
  if (upper !== undefined && lower !== undefined && middle) {
    const width = pct(middle, upper) * 2
    if (width < 6) {
      add({
        name: 'volatility',
        bullish: true,
        strength: 'moderate',
        detail: `bands are tight (${round(width)}% wide), which usually precedes a move`,
      })
    }
    if (last.close > upper) {
      add({ name: 'band break', bullish: true, strength: 'strong', detail: 'closed above the upper band' })
    }
  }
  const drawdown = pct(windowHigh, last.close)
  if (drawdown > -3) {
    add({
      name: 'range position',
      bullish: true,
      strength: drawdown > -1 ? 'strong' : 'moderate',
      detail: `trading within ${round(Math.abs(drawdown))}% of the window high`,
    })
  }

  const bullishWeight = signals.filter((signal) => signal.bullish).reduce((total, signal) => total + WEIGHT[signal.strength], 0)
  const total = signals.reduce((sum, signal) => sum + WEIGHT[signal.strength], 0)
  const bullishScore = total === 0 ? 50 : Math.round((bullishWeight / total) * 100)

  const trend =
    fast !== undefined && slow !== undefined
      ? Math.abs(pct(slow, fast)) < 0.5
        ? 'sideways'
        : fast > slow
          ? 'up'
          : 'down'
      : 'sideways'

  const read: TechnicalRead = {
    timeframe: options.timeframe,
    bars: candles.length,
    price: last.close,
    changePct: round(windowChange),
    trend,
    structure:
      higherHighs && higherLows
        ? 'higher highs, higher lows'
        : lowerHighs && lowerLows
          ? 'lower highs, lower lows'
          : higherLows
            ? 'higher lows'
            : 'no clean structure',
    ema: {
      ...(fast === undefined ? {} : { fast }),
      ...(slow === undefined ? {} : { slow }),
      fastPeriod,
      slowPeriod,
    },
    ...(rsiValue === undefined ? {} : { rsi: round(rsiValue, 1) }),
    macd: {
      ...(latest(momentum.macd) === undefined ? {} : { line: latest(momentum.macd) as number }),
      ...(latest(momentum.signal) === undefined ? {} : { signal: latest(momentum.signal) as number }),
      ...(histogram === undefined ? {} : { histogram }),
    },
    bollinger: {
      ...(upper === undefined ? {} : { upper }),
      ...(lower === undefined ? {} : { lower }),
      ...(upper === undefined || lower === undefined || !middle
        ? {}
        : {
            widthPct: round(((upper - lower) / middle) * 100),
            positionPct: round(((last.close - lower) / (upper - lower)) * 100),
          }),
    },
    ...(atrValue === undefined ? {} : { atrPct: round((atrValue / last.close) * 100) }),
    ...(vwap(candles) === undefined ? {} : { vwap: vwap(candles) as number }),
    volume: {
      last: last.volume,
      average: volumeAverage,
      ratio: volumeAverage > 0 ? round(last.volume / volumeAverage, 2) : 0,
    },
    levels: {
      ...(support ? { support: support.price } : {}),
      ...(resistance ? { resistance: resistance.price } : {}),
      windowHigh,
      windowLow,
    },
    fromHighPct: round(drawdown),
    fromLowPct: round(pct(windowLow, last.close)),
    signals,
    bullishScore,
    notes: [],
  }
  read.notes = describe(read)
  return read
}

/** The read as sentences. Each one is quotable in a post without further checking. */
function describe(read: TechnicalRead): string[] {
  const notes: string[] = [
    `Over the last ${read.bars} ${read.timeframe} bars price is ${read.changePct >= 0 ? 'up' : 'down'} ` +
      `${Math.abs(read.changePct)}%, now at $${read.price.toPrecision(4)}.`,
    `Trend reads ${read.trend}; structure is ${read.structure}.`,
  ]
  if (read.rsi !== undefined) notes.push(`RSI(14) is ${read.rsi}.`)
  if (read.levels.support !== undefined) {
    notes.push(`Nearest support that has actually been defended: $${read.levels.support.toPrecision(4)}.`)
  }
  if (read.levels.resistance !== undefined) {
    notes.push(`Nearest overhead level: $${read.levels.resistance.toPrecision(4)}.`)
  }
  notes.push(
    `Window high $${read.levels.windowHigh.toPrecision(4)}, low $${read.levels.windowLow.toPrecision(4)}; ` +
      `price sits ${Math.abs(read.fromHighPct)}% off the high and ${read.fromLowPct}% off the low.`,
  )
  if (read.volume.ratio > 0) notes.push(`Last bar's volume was ${read.volume.ratio}x the running average.`)
  notes.push(
    `Signal balance: ${read.bullishScore}/100 bullish (${read.signals.filter((s) => s.bullish).length} of ` +
      `${read.signals.length} signals reading up).`,
  )
  return notes
}

function rising(values: number[]): boolean {
  return values.length >= 2 && values.every((value, index) => index === 0 || value > (values[index - 1] as number))
}

function falling(values: number[]): boolean {
  return values.length >= 2 && values.every((value, index) => index === 0 || value < (values[index - 1] as number))
}
