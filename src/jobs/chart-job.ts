import type { Journal } from '../journal/journal.js'
import { isTurn, type Rota } from './cycle.js'
import type { Agent } from '../types.js'

export interface ChartJobDependencies {
  agent: Agent
  symbol: string
  timeframe: string
  logger: { info: (object: unknown, message?: string) => void; error: (object: unknown, message?: string) => void }
  /** Refuses to run again inside this window. Defaults to 20 hours. */
  minIntervalMs?: number
  now?: () => number
  /** Whether post_chart actually publishes; the journal says which. */
  dryRun?: boolean
  /** The posting rota and this job's slot in it. Absent, it posts every day the cron fires. */
  rota?: { rota: Rota; slot: number }
  journal?: Journal
}

/**
 * One chart post a day.
 *
 * The interval guard is not belt and braces: a container that restarts near the
 * cron minute runs the job again, and `node-cron` fires on the schedule of
 * whatever process is alive. Two identical chart posts an hour apart is exactly
 * the repetitive-publishing pattern that cost this account its reach once
 * already, so the job refuses rather than trusting the timer.
 */
export class ChartJob {
  private lastRunAt = 0

  constructor(private readonly dependencies: ChartJobDependencies) {}

  /** Returns false when it declined to run, which is not an error. */
  async run(options: { force?: boolean } = {}): Promise<boolean> {
    const now = (this.dependencies.now ?? Date.now)()
    const { rota } = this.dependencies
    if (!options.force && rota && !isTurn(rota.rota, rota.slot, now)) {
      this.dependencies.logger.info({ slot: rota.slot, rota: rota.rota }, 'chart job: skipping, not its day on the rota')
      return false
    }
    const minimum = this.dependencies.minIntervalMs ?? 20 * 60 * 60 * 1_000
    if (!options.force && this.lastRunAt > 0 && now - this.lastRunAt < minimum) {
      this.dependencies.logger.info(
        { sinceLastRunMs: now - this.lastRunAt, minimum },
        'chart job: skipping, it already ran recently',
      )
      return false
    }
    this.lastRunAt = now

    const task = [
      `Post today's ${this.dependencies.symbol} chart.`,
      `Start on the ${this.dependencies.timeframe} timeframe; use a different one if the read`,
      'is genuinely about a different horizon.',
      'Follow your instructions exactly: research, render once, post once, remember what you did.',
    ].join(' ')

    const result = await this.dependencies.agent.run({
      task,
      threadId: `chart:${new Date(now).toISOString().slice(0, 10)}`,
    })
    this.dependencies.logger.info(
      { tools: result.steps.map((step) => step.tool), output: result.output },
      'chart job: done',
    )
    await this.journal(now, result)
    return true
  }

  /**
   * The chart turn, for the mind page. The caption is whatever was handed to
   * post_chart, so the journal shows what went out rather than the agent's
   * summary of it; the summary is the thinking.
   */
  private async journal(now: number, result: Awaited<ReturnType<Agent['run']>>): Promise<void> {
    const { journal, logger, symbol, timeframe, dryRun } = this.dependencies
    if (!journal) return
    const posted = result.steps.find((step) => step.tool === 'post_chart')
    const caption =
      posted && typeof posted.input === 'object' && posted.input !== null && 'text' in posted.input
        ? String((posted.input as { text: unknown }).text)
        : ''
    try {
      await journal.record({
        id: `chart:${new Date(now).toISOString().slice(0, 10)}`,
        at: new Date(now).toISOString(),
        trigger: 'chart',
        incoming: `daily ${symbol} chart, ${timeframe}`,
        tone: 'neutral',
        recalled: [],
        tools: result.steps.map((step) => ({ name: step.tool, result: step.result ?? '' })),
        thinking: result.output,
        said: caption,
        declined: [],
        outcome: posted ? (dryRun ? 'dry-run' : 'posted') : 'skipped',
        ...(posted ? {} : { outcomeNote: 'the agent finished without posting' }),
      })
    } catch (error) {
      logger.error({ err: error }, 'chart job: could not journal the turn')
    }
  }
}
