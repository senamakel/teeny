import type { Journal } from '../journal/journal.js'
import type { MoonResult } from '../moon/pipeline.js'
import { multiple } from '../moon/svg.js'

export interface MoonJobDependencies {
  pipeline: { run(): Promise<MoonResult> }
  logger: { info: (object: unknown, message?: string) => void; error: (object: unknown, message?: string) => void }
  /** Refuses to run again inside this window. Defaults to 20 hours. */
  minIntervalMs?: number
  now?: () => number
  journal?: Journal
}

/**
 * One moon chart a day.
 *
 * The same interval guard as the chart and morning jobs, for the same
 * reason: a container restarted near the cron minute would post the same
 * picture twice, and two identical posts an hour apart is the pattern that
 * got this account throttled.
 */
export class MoonJob {
  private lastRunAt = 0

  constructor(private readonly dependencies: MoonJobDependencies) {}

  /** Returns the result, or undefined when it declined to run, which is not an error. */
  async run(options: { force?: boolean } = {}): Promise<MoonResult | undefined> {
    const now = (this.dependencies.now ?? Date.now)()
    const minimum = this.dependencies.minIntervalMs ?? 20 * 60 * 60 * 1_000
    if (!options.force && this.lastRunAt > 0 && now - this.lastRunAt < minimum) {
      this.dependencies.logger.info({ sinceLastRunMs: now - this.lastRunAt, minimum }, 'moon job: skipping, it already ran recently')
      return undefined
    }
    this.lastRunAt = now

    let result: MoonResult
    try {
      result = await this.dependencies.pipeline.run()
    } catch (error) {
      await this.journalFailure(now, error)
      throw error
    }
    this.dependencies.logger.info(
      {
        outcome: result.outcome,
        yardstick: result.target.coin.symbol,
        multiple: result.target.multiple,
        problems: result.problems,
        postedId: result.postedId,
      },
      'moon job: done',
    )
    await this.journal(now, result)
    return result
  }

  private id(now: number): string {
    return `moon:${new Date(now).toISOString().slice(0, 10)}`
  }

  private async journal(now: number, result: MoonResult): Promise<void> {
    const { journal, logger } = this.dependencies
    if (!journal) return
    try {
      await journal.record({
        id: this.id(now),
        at: new Date(now).toISOString(),
        trigger: 'moon',
        incoming: `${result.board.length} coins on the board, $${result.token.symbol} at $${Math.round(result.token.marketCapUsd).toLocaleString('en-US')}`,
        tone: 'neutral',
        recalled: [result.recalled.history, result.recalled.community].filter(Boolean),
        tools: [
          { name: 'leaderboard', result: result.board.map((coin) => coin.symbol).join(', ') },
          { name: 'render_moon', result: result.path ?? `${result.png.byteLength} bytes` },
        ],
        thinking: [
          `yardstick: $${result.target.coin.symbol} (${result.target.coin.name})`,
          `multiple: ${multiple(result.target.multiple)}`,
          `implied: $${result.target.impliedPriceUsd}`,
          ...(result.excluded.length > 0 ? [`skipped as recently used: ${result.excluded.join(', ')}`] : []),
          `ladder: ${result.ladder.map((entry) => `$${entry.coin.symbol} ${multiple(entry.multiple)}`).join(', ')}`,
        ].join('\n'),
        // A held caption is in the hold log for review, never on the public page.
        said: result.outcome === 'held' ? '' : result.caption,
        declined: result.problems,
        outcome: result.outcome,
        ...(result.outcome === 'held'
          ? { outcomeNote: result.problems.find((problem) => problem.startsWith('content filter withheld')) ?? 'content filter withheld the caption' }
          : {}),
      })
    } catch (error) {
      logger.error({ err: error }, 'moon job: could not journal the turn')
    }
  }

  private async journalFailure(now: number, error: unknown): Promise<void> {
    const { journal, logger } = this.dependencies
    if (!journal) return
    try {
      await journal.record({
        id: this.id(now),
        at: new Date(now).toISOString(),
        trigger: 'moon',
        incoming: 'the moon chart',
        tone: 'neutral',
        recalled: [],
        tools: [],
        thinking: '',
        said: '',
        declined: [],
        outcome: 'failed',
        outcomeNote: error instanceof Error ? error.message : String(error),
      })
    } catch (journalError) {
      logger.error({ err: journalError }, 'moon job: could not journal the failure')
    }
  }
}
