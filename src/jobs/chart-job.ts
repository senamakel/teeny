import type { Agent } from '../types.js'

export interface ChartJobDependencies {
  agent: Agent
  symbol: string
  timeframe: string
  logger: { info: (object: unknown, message?: string) => void; error: (object: unknown, message?: string) => void }
  /** Refuses to run again inside this window. Defaults to 20 hours. */
  minIntervalMs?: number
  now?: () => number
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
    return true
  }
}
