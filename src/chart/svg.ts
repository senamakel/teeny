import type { Candle, Timeframe } from '../market/types.js'
import type { Series } from '../ta/indicators.js'
import { axisTime, escapeXml, price, signedPercent } from './format.js'
import { THEME } from './theme.js'

export interface Overlay {
  label: string
  color: string
  /** Aligned to `candles`; `undefined` where the indicator has no value yet. */
  values: Series
}

export interface HorizontalLine {
  label: string
  value: number
  color?: string
}

export interface ChartInput {
  candles: Candle[]
  symbol: string
  name?: string
  timeframe: Timeframe
  /** The live price, which may be newer than the last candle's close. */
  priceUsd?: number
  changePct?: number
  overlays?: Overlay[]
  /** Support/resistance and similar, drawn as dashed rules. */
  lines?: HorizontalLine[]
  /** The strip under the title: market cap, liquidity, volume. */
  stats?: Array<{ label: string; value: string }>
  watermark?: string
  width?: number
  height?: number
}

const PADDING = { top: 92, right: 96, bottom: 36, left: 16 }
const VOLUME_FRACTION = 0.18
const GAP = 16

/**
 * A candlestick chart as SVG text.
 *
 * SVG rather than a drawing library because it is a string: the tests assert on
 * it, a diff shows what changed, and the only native dependency in the whole
 * path is the rasteriser that turns it into the PNG X actually accepts.
 */
export function renderChartSvg(input: ChartInput): string {
  const candles = input.candles
  if (candles.length === 0) throw new Error('Cannot draw a chart with no candles')

  const width = input.width ?? 1_200
  const height = input.height ?? 675
  const plotLeft = PADDING.left
  const plotRight = width - PADDING.right
  const plotWidth = plotRight - plotLeft
  const volumeHeight = (height - PADDING.top - PADDING.bottom) * VOLUME_FRACTION
  const priceTop = PADDING.top
  const priceBottom = height - PADDING.bottom - volumeHeight - GAP
  const priceHeight = priceBottom - priceTop
  const volumeBottom = height - PADDING.bottom

  // The vertical scale covers the overlays too. An EMA that leaves the panel
  // is worse than no EMA: it reads as the price having gone somewhere it did
  // not.
  const overlayValues = (input.overlays ?? []).flatMap((overlay) =>
    overlay.values.flatMap((value) => (value === undefined ? [] : [value])),
  )
  const lineValues = (input.lines ?? []).map((line) => line.value)
  const highest = Math.max(...candles.map((candle) => candle.high), ...overlayValues, ...lineValues)
  const lowest = Math.min(...candles.map((candle) => candle.low), ...overlayValues, ...lineValues)
  const span = highest - lowest || highest || 1
  const top = highest + span * 0.08
  const bottom = Math.max(lowest - span * 0.08, 0)

  const y = (value: number): number =>
    priceBottom - ((value - bottom) / (top - bottom || 1)) * priceHeight
  const slot = plotWidth / candles.length
  const bodyWidth = Math.max(Math.min(slot * 0.68, 22), 1)
  const x = (index: number): number => plotLeft + slot * (index + 0.5)

  // The volume panel is scaled to the 95th percentile, not to the maximum.
  // One launch bar or one whale print is routinely fifty times the median, and
  // scaling to it flattens every other bar into the axis -- a volume panel that
  // shows one spike and nothing else tells you less than no panel at all. Bars
  // over the cap are clipped to the top of the panel, which reads correctly:
  // that bar is off the scale.
  const volumeCap = percentile(candles.map((candle) => candle.volume), 0.95) * 1.25 || 1
  const volumeY = (value: number): number =>
    volumeBottom - (Math.min(value, volumeCap) / volumeCap) * volumeHeight

  const last = candles[candles.length - 1] as Candle
  const shownPrice = input.priceUsd ?? last.close
  const change = input.changePct
  const changeColor = change === undefined ? THEME.muted : change >= 0 ? THEME.up : THEME.down

  const parts: string[] = [
    `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}" font-family="${THEME.font}">`,
    `<rect width="${width}" height="${height}" fill="${THEME.background}"/>`,
  ]

  // Watermark first, so every later element sits on top of it.
  if (input.watermark) {
    parts.push(
      `<text x="${plotLeft + plotWidth / 2}" y="${priceTop + priceHeight / 2 + 24}" fill="${THEME.watermark}" font-size="72" font-weight="700" text-anchor="middle">${escapeXml(input.watermark)}</text>`,
    )
  }

  // Horizontal grid and the price axis on the right, where a chart puts it.
  const ticks = priceTicks(bottom, top, 6)
  for (const tick of ticks) {
    const line = y(tick)
    parts.push(
      `<line x1="${plotLeft}" y1="${line}" x2="${plotRight}" y2="${line}" stroke="${THEME.grid}" stroke-width="1"/>`,
      `<text x="${plotRight + 8}" y="${line + 4}" fill="${THEME.muted}" font-size="13">${escapeXml(price(tick))}</text>`,
    )
  }
  parts.push(
    `<line x1="${plotRight}" y1="${priceTop}" x2="${plotRight}" y2="${volumeBottom}" stroke="${THEME.axis}" stroke-width="1"/>`,
    `<line x1="${plotLeft}" y1="${volumeBottom}" x2="${plotRight}" y2="${volumeBottom}" stroke="${THEME.axis}" stroke-width="1"/>`,
  )

  // Time axis: about six labels, whatever the bar count.
  const step = Math.max(Math.floor(candles.length / 6), 1)
  for (let index = 0; index < candles.length; index += step) {
    const candle = candles[index] as Candle
    const at = x(index)
    parts.push(
      `<line x1="${at}" y1="${priceTop}" x2="${at}" y2="${volumeBottom}" stroke="${THEME.grid}" stroke-width="1"/>`,
      `<text x="${at}" y="${volumeBottom + 20}" fill="${THEME.muted}" font-size="12" text-anchor="middle">${escapeXml(axisTime(candle.time, input.timeframe))}</text>`,
    )
  }

  for (const line of input.lines ?? []) {
    const at = y(line.value)
    if (at < priceTop || at > priceBottom) continue
    parts.push(
      `<line x1="${plotLeft}" y1="${at}" x2="${plotRight}" y2="${at}" stroke="${line.color ?? THEME.level}" stroke-width="1" stroke-dasharray="6 5"/>`,
      `<text x="${plotLeft + 8}" y="${at - 6}" fill="${line.color ?? THEME.muted}" font-size="12">${escapeXml(line.label)}</text>`,
    )
  }

  // Volume, under the price panel and behind nothing.
  for (const [index, candle] of candles.entries()) {
    const height_ = Math.max(volumeBottom - volumeY(candle.volume), 0)
    parts.push(
      `<rect x="${x(index) - bodyWidth / 2}" y="${volumeY(candle.volume)}" width="${bodyWidth}" height="${height_}" fill="${candle.close >= candle.open ? THEME.upVolume : THEME.downVolume}"/>`,
    )
  }

  // Candles. The wick is drawn first so the body covers its middle.
  for (const [index, candle] of candles.entries()) {
    const up = candle.close >= candle.open
    const colour = up ? THEME.up : THEME.down
    const at = x(index)
    const bodyTop = y(Math.max(candle.open, candle.close))
    // A doji has a zero-height body, which renders as nothing at all; one
    // pixel is what makes an unchanged bar still visible.
    const bodyHeight = Math.max(y(Math.min(candle.open, candle.close)) - bodyTop, 1)
    parts.push(
      `<line x1="${at}" y1="${y(candle.high)}" x2="${at}" y2="${y(candle.low)}" stroke="${colour}" stroke-width="1.5"/>`,
      `<rect x="${at - bodyWidth / 2}" y="${bodyTop}" width="${bodyWidth}" height="${bodyHeight}" fill="${colour}"/>`,
    )
  }

  for (const overlay of input.overlays ?? []) {
    const path = polyline(overlay.values, x, y)
    if (!path) continue
    parts.push(
      `<path d="${path}" fill="none" stroke="${overlay.color}" stroke-width="1.8" stroke-linejoin="round" stroke-linecap="round" opacity="0.9"/>`,
    )
  }

  // The last-price tag on the axis, the one thing everybody actually reads.
  const tagY = y(last.close)
  const tagColour = last.close >= last.open ? THEME.up : THEME.down
  parts.push(
    `<line x1="${plotLeft}" y1="${tagY}" x2="${plotRight}" y2="${tagY}" stroke="${tagColour}" stroke-width="1" stroke-dasharray="3 3" opacity="0.7"/>`,
    `<rect x="${plotRight + 1}" y="${tagY - 11}" width="${PADDING.right - 6}" height="22" rx="3" fill="${tagColour}"/>`,
    `<text x="${plotRight + 8}" y="${tagY + 4}" fill="#0b0e11" font-size="13" font-weight="700">${escapeXml(price(last.close))}</text>`,
  )

  // Header.
  parts.push(
    `<text x="${plotLeft}" y="34" fill="${THEME.text}" font-size="26" font-weight="700">${escapeXml(input.symbol)}</text>`,
    `<text x="${plotLeft + input.symbol.length * 16 + 12}" y="34" fill="${THEME.muted}" font-size="15">${escapeXml(`${input.name ?? ''} · ${input.timeframe} · ${candles.length} bars`)}</text>`,
    `<text x="${plotLeft}" y="64" fill="${THEME.text}" font-size="22" font-weight="600">${escapeXml(`$${price(shownPrice)}`)}</text>`,
  )
  if (change !== undefined) {
    parts.push(
      `<text x="${plotLeft + 130}" y="64" fill="${changeColor}" font-size="18" font-weight="600">${escapeXml(signedPercent(change))}</text>`,
    )
  }

  let offset = plotLeft + 230
  for (const stat of input.stats ?? []) {
    const text = `${stat.label} ${stat.value}`
    parts.push(
      `<text x="${offset}" y="64" fill="${THEME.muted}" font-size="15">${escapeXml(text)}</text>`,
    )
    offset += text.length * 8.2 + 26
  }

  // The overlay legend goes top-right, out of the way of the price.
  let legendX = plotRight
  for (const overlay of [...(input.overlays ?? [])].reverse()) {
    legendX -= overlay.label.length * 8 + 22
    parts.push(
      `<text x="${legendX}" y="${priceTop - 10}" fill="${overlay.color}" font-size="13" font-weight="600">${escapeXml(overlay.label)}</text>`,
    )
  }

  parts.push('</svg>')
  return parts.join('\n')
}

/** `undefined` breaks the line rather than interpolating across the gap. */
function polyline(values: Series, x: (index: number) => number, y: (value: number) => number): string {
  const segments: string[] = []
  let drawing = false
  values.forEach((value, index) => {
    if (value === undefined) {
      drawing = false
      return
    }
    segments.push(`${drawing ? 'L' : 'M'}${x(index).toFixed(2)} ${y(value).toFixed(2)}`)
    drawing = true
  })
  return segments.join(' ')
}

/** The value below which `fraction` of the sorted inputs fall. */
export function percentile(values: number[], fraction: number): number {
  if (values.length === 0) return 0
  const sorted = [...values].sort((a, b) => a - b)
  const index = Math.min(Math.floor(sorted.length * fraction), sorted.length - 1)
  return sorted[index] as number
}

/** Round-ish gridline values inside the range, in the token's own order of magnitude. */
export function priceTicks(low: number, high: number, count: number): number[] {
  if (!(high > low)) return [low]
  const raw = (high - low) / count
  const magnitude = 10 ** Math.floor(Math.log10(raw))
  const normalised = raw / magnitude
  const step = (normalised >= 5 ? 5 : normalised >= 2 ? 2 : 1) * magnitude
  const ticks: number[] = []
  for (let value = Math.ceil(low / step) * step; value <= high; value += step) {
    ticks.push(Number(value.toPrecision(12)))
  }
  return ticks
}
