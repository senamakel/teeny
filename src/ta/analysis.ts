import type { Candle, Timeframe } from '../market/types.js'
import { atr, bollinger, closes, ema, latest, levels, sma, swings, vwap } from './indicators.js'

export type Strength = 'weak' | 'moderate' | 'strong'

export interface Signal {
  name: string
  /** Which way it reads. A bearish signal is still reported: see `analyse`. */
  bullish: boolean
  strength: Strength
  detail: string
}

/** A level that has just been crossed, which is the only kind worth a post. */
export interface LevelEvent {
  kind: 'breakout' | 'breakdown' | 'reclaim'
  level: number
  touches: number
  /** Whether the bar that did it traded above its own average volume. */
  volumeConfirmed: boolean
  barsAgo: number
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
  /** ATR as a percentage of price, and how tight the range has become. */
  volatility: { atrPct?: number; rangeWidthPct?: number }
  vwap?: number
  volume: {
    last: number
    average: number
    ratio: number
    /** Recent half of the window against the half before it. Above 1 is waking up. */
    trend: number
  }
  levels: {
    support?: number
    supportTouches?: number
    resistance?: number
    resistanceTouches?: number
    windowHigh: number
    windowLow: number
  }
  event?: LevelEvent
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
 * There are deliberately no oscillators here. RSI and MACD were in the first
 * version and came out again: they are mean-reversion tools built for
 * instruments that mean-revert, and a memecoin does not. RSI sat at 74 and read
 * "overbought" through a move that was the whole point of the day, which is
 * worse than useless in a post -- it is a caveat that talks holders out of
 * their own thesis on the strength of a number that never meant anything here.
 *
 * What is left is what actually moves one of these: levels that have been
 * defended more than once, what happens when one breaks, whether volume showed
 * up for it, and where price sits against its own range. Trend still uses two
 * moving averages, which are a description of where price has been rather than
 * a prediction of where it goes next.
 *
 * Bearish signals are reported even though the account that reads this is
 * unapologetically long. A bullish poster that cannot see a downtrend
 * eventually posts "accumulation zone" into a chart that is bleeding, and the
 * timeline notices.
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

  const fast = latest(ema(values, fastPeriod))
  const slow = latest(ema(values, slowPeriod))
  const bands = bollinger(values, 20, 2)
  const atrValue = latest(atr(candles, 14))
  const volumes = candles.map((candle) => candle.volume)
  const volumeAverage = latest(sma(volumes, Math.min(20, candles.length))) ?? 0

  const windowHigh = Math.max(...candles.map((candle) => candle.high))
  const windowLow = Math.min(...candles.map((candle) => candle.low))
  const windowChange = pct(first.open, last.close)

  const upper = latest(bands.upper)
  const lower = latest(bands.lower)
  const middle = latest(bands.middle)

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

  const event = findEvent(candles, found, volumeAverage)
  const volumeTrend = trendOf(volumes)

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

  // A level breaking is the single most postable thing that happens on one of
  // these charts, so it is weighted as such -- and downgraded without volume,
  // because a break nobody traded is a wick that gets given straight back.
  if (event) {
    add({
      name: event.kind,
      bullish: event.kind !== 'breakdown',
      strength: event.volumeConfirmed ? 'strong' : 'moderate',
      detail:
        `${event.kind === 'breakdown' ? 'lost' : event.kind === 'reclaim' ? 'reclaimed' : 'broke above'} ` +
        `$${event.level.toPrecision(4)} (${event.touches} touches) ${event.barsAgo === 0 ? 'this bar' : `${event.barsAgo} bars ago`}` +
        `${event.volumeConfirmed ? ', on above-average volume' : ', but volume did not confirm'}`,
    })
  }

  if (support) {
    add({
      name: 'support',
      bullish: true,
      strength: support.touches >= 3 ? 'strong' : 'moderate',
      detail: `$${support.price.toPrecision(4)} has been defended ${support.touches} times and is still below price`,
    })
  }

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
        name: 'volume spike',
        bullish: last.close >= last.open,
        strength: ratio >= 3 ? 'strong' : 'moderate',
        detail: `last bar traded ${round(ratio, 1)}x the average volume`,
      })
    }
  }
  if (volumeTrend >= 1.3 || volumeTrend <= 0.7) {
    add({
      name: 'volume trend',
      bullish: volumeTrend >= 1.3,
      strength: volumeTrend >= 2 || volumeTrend <= 0.5 ? 'strong' : 'moderate',
      detail:
        volumeTrend >= 1.3
          ? `volume in the recent half of the window is ${round(volumeTrend, 1)}x the half before it`
          : `volume has dried up to ${round(volumeTrend, 1)}x the earlier half of the window`,
    })
  }

  // A tight range is not bullish by itself. It is a setup: price is coiling,
  // and the next real move tends to come out of one.
  const rangeWidth =
    upper !== undefined && lower !== undefined && middle ? ((upper - lower) / middle) * 100 : undefined
  if (rangeWidth !== undefined && rangeWidth < 6) {
    add({
      name: 'range tightening',
      bullish: true,
      strength: 'moderate',
      detail: `the range has narrowed to ${round(rangeWidth)}% wide, which is where moves start`,
    })
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

  const bullishWeight = signals
    .filter((signal) => signal.bullish)
    .reduce((total, signal) => total + WEIGHT[signal.strength], 0)
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
    volatility: {
      ...(atrValue === undefined ? {} : { atrPct: round((atrValue / last.close) * 100) }),
      ...(rangeWidth === undefined ? {} : { rangeWidthPct: round(rangeWidth) }),
    },
    ...(vwap(candles) === undefined ? {} : { vwap: vwap(candles) as number }),
    volume: {
      last: last.volume,
      average: volumeAverage,
      ratio: volumeAverage > 0 ? round(last.volume / volumeAverage, 2) : 0,
      trend: round(volumeTrend, 2),
    },
    levels: {
      ...(support ? { support: support.price, supportTouches: support.touches } : {}),
      ...(resistance ? { resistance: resistance.price, resistanceTouches: resistance.touches } : {}),
      windowHigh,
      windowLow,
    },
    ...(event ? { event } : {}),
    fromHighPct: round(drawdown),
    fromLowPct: round(pct(windowLow, last.close)),
    signals,
    bullishScore,
    notes: [],
  }
  read.notes = describe(read)
  return read
}

/**
 * The most recent level crossing, if there was one worth mentioning.
 *
 * Only levels with more than one touch count. A line price has been to once is
 * not a level, it is a coordinate, and "broke resistance" about one of those is
 * the kind of claim that makes a chart account unreadable.
 */
function findEvent(
  candles: Candle[],
  found: Array<{ price: number; touches: number }>,
  volumeAverage: number,
): LevelEvent | undefined {
  const real = found.filter((level) => level.touches >= 2)
  if (real.length === 0) return undefined

  // Only the recent past: a break twelve bars ago is history, not news.
  const lookback = Math.min(12, candles.length - 1)
  for (let back = 0; back < lookback; back += 1) {
    const index = candles.length - 1 - back
    const candle = candles[index]
    const previous = candles[index - 1]
    if (!candle || !previous) continue

    for (const level of real) {
      const crossedUp = previous.close <= level.price && candle.close > level.price
      const crossedDown = previous.close >= level.price && candle.close < level.price
      if (!crossedUp && !crossedDown) continue

      // Reclaim rather than breakout when price had been under this level for a
      // while and has come back over it: the two read completely differently to
      // somebody who has been holding through the part where it was lost.
      const lostEarlier = candles
        .slice(Math.max(0, index - 24), index)
        .some((bar) => bar.close < level.price)
      return {
        kind: crossedDown ? 'breakdown' : lostEarlier ? 'reclaim' : 'breakout',
        level: level.price,
        touches: level.touches,
        volumeConfirmed: volumeAverage > 0 && candle.volume > volumeAverage,
        barsAgo: back,
      }
    }
  }
  return undefined
}

/** Volume in the recent half of the window against the half before it. */
function trendOf(volumes: number[]): number {
  if (volumes.length < 4) return 1
  const half = Math.floor(volumes.length / 2)
  const mean = (values: number[]): number =>
    values.length === 0 ? 0 : values.reduce((total, value) => total + value, 0) / values.length
  const earlier = mean(volumes.slice(0, half))
  const recent = mean(volumes.slice(half))
  return earlier === 0 ? (recent > 0 ? 2 : 1) : recent / earlier
}

/** The read as sentences. Each one is quotable in a post without further checking. */
function describe(read: TechnicalRead): string[] {
  const notes: string[] = [
    `Over the last ${read.bars} ${read.timeframe} bars price is ${read.changePct >= 0 ? 'up' : 'down'} ` +
      `${Math.abs(read.changePct)}%, now at $${read.price.toPrecision(4)}.`,
    `Trend reads ${read.trend}; structure is ${read.structure}.`,
  ]
  if (read.event) {
    notes.push(
      `${read.event.kind === 'breakdown' ? 'Lost' : read.event.kind === 'reclaim' ? 'Reclaimed' : 'Broke above'} ` +
        `$${read.event.level.toPrecision(4)} ${read.event.barsAgo === 0 ? 'on the current bar' : `${read.event.barsAgo} bars ago`}` +
        `${read.event.volumeConfirmed ? ', with volume behind it.' : ', but volume did not confirm it.'}`,
    )
  }
  if (read.levels.support !== undefined) {
    notes.push(
      `Nearest support that has actually been defended: $${read.levels.support.toPrecision(4)}` +
        `${read.levels.supportTouches ? ` (${read.levels.supportTouches} touches)` : ''}.`,
    )
  }
  if (read.levels.resistance !== undefined) {
    notes.push(
      `Nearest overhead level: $${read.levels.resistance.toPrecision(4)}` +
        `${read.levels.resistanceTouches ? ` (${read.levels.resistanceTouches} touches)` : ''}.`,
    )
  }
  notes.push(
    `Window high $${read.levels.windowHigh.toPrecision(4)}, low $${read.levels.windowLow.toPrecision(4)}; ` +
      `price sits ${Math.abs(read.fromHighPct)}% off the high and ${read.fromLowPct}% off the low.`,
  )
  notes.push(
    `Volume: last bar ${read.volume.ratio}x its average, and the recent half of the window is ` +
      `${read.volume.trend}x the half before it.`,
  )
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
