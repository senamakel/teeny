import { mkdirSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import type { Candle, Timeframe, TokenOverview } from '../market/types.js'
import type { TechnicalRead } from '../ta/analysis.js'
import { renderChart, type RenderedChart } from './chart.js'

export interface ChartJob {
  candles: Candle[]
  timeframe: Timeframe
  overview?: TokenOverview
  read?: TechnicalRead
  symbol?: string
  watermark?: string
  outputDir?: string
}

export interface ChartRenderer {
  readonly name: string
  render(job: ChartJob): Promise<RenderedChart>
}

/** Draws the chart here, from candles this service fetched. The default. */
export class NativeChartRenderer implements ChartRenderer {
  readonly name = 'native'

  async render(job: ChartJob): Promise<RenderedChart> {
    return renderChart({
      candles: job.candles,
      timeframe: job.timeframe,
      ...(job.overview ? { overview: job.overview } : {}),
      ...(job.read ? { read: job.read } : {}),
      ...(job.symbol ? { symbol: job.symbol } : {}),
      ...(job.watermark ? { watermark: job.watermark } : {}),
      ...(job.outputDir ? { outputDir: job.outputDir } : {}),
    })
  }
}

export interface ScreenshotOptions {
  /**
   * `{pair}` and `{chain}` are substituted. Defaults to the DEX Screener embed.
   * Point it at any page that draws a chart -- including a locally served one.
   */
  urlTemplate?: string
  chain?: string
  /** A Playwright browser channel, e.g. `chrome`, or leave unset for the bundled one. */
  channel?: string
  executablePath?: string
  width?: number
  height?: number
  /** How long to let the chart's own data feed fill in before the shutter. */
  settleMs?: number
  timeoutMs?: number
  /** Falls back to the native renderer rather than failing the post. Default true. */
  fallback?: ChartRenderer | false
}

const DEFAULT_URL =
  'https://dexscreener.com/{chain}/{pair}?embed=1&theme=dark&trades=0&info=0&interval=60'

/**
 * Screenshots somebody else's chart.
 *
 * Worth knowing before turning this on: as of the day it was written, DEX
 * Screener's embed does not serve its chart data to a headless browser. The
 * page loads, the TradingView shell renders, the toolbar is all there, and the
 * panel says "No data here" -- verified against the $TINY PumpSwap pool with a
 * real Chrome binary, a reload, and generous waits. So this is off by default and the
 * account draws its own charts instead.
 *
 * It is kept because the URL is a template, and the useful version of this idea
 * is not screenshotting DEX Screener at all: it is pointing this at a page we
 * serve ourselves, with a real charting library fed by our own candles. That
 * needs a browser in the runtime image, which is why it is opt-in.
 */
export class ScreenshotChartRenderer implements ChartRenderer {
  readonly name = 'screenshot'
  private readonly fallback: ChartRenderer | undefined

  constructor(private readonly options: ScreenshotOptions = {}) {
    this.fallback =
      options.fallback === false ? undefined : (options.fallback ?? new NativeChartRenderer())
  }

  async render(job: ChartJob): Promise<RenderedChart> {
    try {
      return await this.shoot(job)
    } catch (error) {
      if (!this.fallback) throw error
      return this.fallback.render(job)
    }
  }

  private async shoot(job: ChartJob): Promise<RenderedChart> {
    const pair = job.overview?.poolAddress
    if (!pair) throw new Error('A screenshot needs a pool address and the overview has none')

    // Imported here rather than at the top: playwright pulls in a browser
    // driver, and the default path through this service never uses it.
    const { chromium } = await import('playwright-core')
    const width = this.options.width ?? 1_200
    const height = this.options.height ?? 675
    const browser = await chromium.launch({
      headless: true,
      ...(this.options.channel ? { channel: this.options.channel } : {}),
      ...(this.options.executablePath ? { executablePath: this.options.executablePath } : {}),
    })
    try {
      const page = await browser.newPage({
        viewport: { width, height },
        deviceScaleFactor: 2,
      })
      const url = (this.options.urlTemplate ?? DEFAULT_URL)
        .replace('{pair}', pair)
        .replace('{chain}', this.options.chain ?? job.overview?.chain ?? 'solana')
      // `networkidle` never arrives on a live chart -- the price feed is a
      // socket that keeps talking -- so the wait is explicit instead.
      await page.goto(url, {
        waitUntil: 'domcontentloaded',
        timeout: this.options.timeoutMs ?? 45_000,
      })
      await page.waitForTimeout(this.options.settleMs ?? 10_000)
      const png = await page.screenshot({ type: 'png' })

      const rendered: RenderedChart = { svg: '', png, width, height }
      if (job.outputDir) {
        const name = `${(job.symbol ?? 'token').replace(/[^A-Za-z0-9]/g, '')}-${job.timeframe}-${Date.now()}.png`
        mkdirSync(job.outputDir, { recursive: true })
        const path = join(job.outputDir, name)
        writeFileSync(path, png)
        rendered.path = path
      }
      return rendered
    } finally {
      await browser.close()
    }
  }
}
