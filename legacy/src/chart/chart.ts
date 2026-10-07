import { mkdirSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import type { Candle, Timeframe, TokenOverview } from '../market/types.js'
import { ema } from '../ta/indicators.js'
import type { TechnicalRead } from '../ta/analysis.js'
import { compactUsd } from './format.js'
import { svgToPng } from './png.js'
import { renderChartSvg, type ChartInput } from './svg.js'
import { THEME } from './theme.js'

export interface RenderedChart {
  svg: string
  png: Buffer
  /** Where the PNG was written, when an output directory was given. */
  path?: string
  width: number
  height: number
}

export interface ChartRequest {
  candles: Candle[]
  timeframe: Timeframe
  overview?: TokenOverview
  read?: TechnicalRead
  symbol?: string
  watermark?: string
  /** Written here as `<symbol>-<timeframe>-<timestamp>.png` when set. */
  outputDir?: string
  width?: number
  height?: number
}

/**
 * The whole chart pipeline: candles in, a PNG on disk out.
 *
 * The overlays and the header strip are derived here rather than passed in, so
 * every chart this account posts is drawn the same way -- same two EMAs, same
 * stats, same levels. A chart whose indicators change from post to post is a
 * chart nobody can read at a glance, which defeats the point of posting one.
 */
export function renderChart(request: ChartRequest): RenderedChart {
  const { candles } = request
  if (candles.length === 0) throw new Error('Cannot render a chart with no candles')

  const values = candles.map((candle) => candle.close)
  const fastPeriod = request.read?.ema.fastPeriod ?? 20
  const slowPeriod = request.read?.ema.slowPeriod ?? 50
  const symbol = request.symbol ?? request.overview?.symbol ?? 'TOKEN'
  const last = candles[candles.length - 1] as Candle
  const first = candles[0] as Candle

  const stats: Array<{ label: string; value: string }> = []
  if (request.overview?.marketCapUsd !== undefined) {
    stats.push({ label: 'MC', value: compactUsd(request.overview.marketCapUsd) })
  } else if (request.overview?.fdvUsd !== undefined) {
    stats.push({ label: 'FDV', value: compactUsd(request.overview.fdvUsd) })
  }
  if (request.overview?.liquidityUsd !== undefined) {
    stats.push({ label: 'LIQ', value: compactUsd(request.overview.liquidityUsd) })
  }
  if (request.overview?.volume24hUsd !== undefined) {
    stats.push({ label: '24H VOL', value: compactUsd(request.overview.volume24hUsd) })
  }

  const lines = [
    ...(request.read?.levels.support === undefined
      ? []
      : [{ label: 'support', value: request.read.levels.support, color: THEME.up }]),
    ...(request.read?.levels.resistance === undefined
      ? []
      : [{ label: 'resistance', value: request.read.levels.resistance, color: THEME.down }]),
  ]

  const input: ChartInput = {
    candles,
    symbol: symbol.startsWith('$') ? symbol : `$${symbol}`,
    ...(request.overview?.name ? { name: request.overview.name } : {}),
    timeframe: request.timeframe,
    ...(request.overview?.priceUsd === undefined ? {} : { priceUsd: request.overview.priceUsd }),
    changePct:
      request.read?.changePct ??
      (first.open === 0 ? 0 : ((last.close - first.open) / first.open) * 100),
    overlays: [
      { label: `EMA${fastPeriod}`, color: THEME.fast, values: ema(values, fastPeriod) },
      { label: `EMA${slowPeriod}`, color: THEME.slow, values: ema(values, slowPeriod) },
    ],
    lines,
    stats,
    ...(request.watermark ? { watermark: request.watermark } : {}),
    ...(request.width === undefined ? {} : { width: request.width }),
    ...(request.height === undefined ? {} : { height: request.height }),
  }

  const svg = renderChartSvg(input)
  const png = svgToPng(svg)
  const rendered: RenderedChart = {
    svg,
    png,
    width: input.width ?? 1_200,
    height: input.height ?? 675,
  }

  if (request.outputDir) {
    const name = `${symbol.replace(/[^A-Za-z0-9]/g, '')}-${request.timeframe}-${Date.now()}.png`
    const path = join(request.outputDir, name)
    mkdirSync(dirname(path), { recursive: true })
    writeFileSync(path, png)
    rendered.path = path
  }
  return rendered
}
