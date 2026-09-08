import type { FastifyBaseLogger } from 'fastify'
import type { ReplyJob } from './reply-job.js'

/**
 * Drains the mention queue continuously instead of on a timer.
 *
 * The webhook puts a mention in the queue within seconds of somebody posting
 * it. Under a cron the reply then waited for the next tick, so the account's
 * answer was up to five minutes behind a conversation that was still happening
 * -- fine for a quiet week, wrong for a giveaway where people are replying in
 * real time and expecting a reply back.
 *
 * This is a poll loop rather than a subscription because the queue is a plain
 * MongoDB collection: change streams need a replica set and this is a single
 * node. So the loop asks immediately again whenever it found work, and waits a
 * short idle interval when it did not, which behaves like a consumer while
 * costing one indexed query per idle tick.
 *
 * It never runs two batches at once -- ReplyJob refuses an overlapping run, and
 * the loop is sequential anyway -- so the ordering and the rate limits stay
 * exactly as they were. What changes is only how long a queued mention waits.
 */
export class ReplyWorker {
  private stopped = false
  private running?: Promise<void>
  /** Wakes the idle wait early, so a stop is immediate rather than up to one
   * idle interval away -- which on a container being replaced is the
   * difference between a clean handover and docker killing it. */
  private wake: (() => void) | undefined

  constructor(
    private readonly dependencies: {
      job: ReplyJob
      logger: FastifyBaseLogger
      /** How long to wait after finding nothing. */
      idleMs: number
      /** How long to wait after a failure, so a broken model does not spin. */
      errorMs?: number
    },
  ) {}

  start(): void {
    this.running ??= this.loop()
  }

  /** Lets the current batch finish rather than cutting a reply in half, but
   * does not sit through the idle wait to do it. */
  async stop(): Promise<void> {
    this.stopped = true
    this.wake?.()
    await this.running
  }

  private async loop(): Promise<void> {
    const { job, logger, idleMs } = this.dependencies
    const errorMs = this.dependencies.errorMs ?? Math.max(idleMs, 5_000)
    logger.info({ idleMs }, 'reply worker draining the queue continuously')

    while (!this.stopped) {
      try {
        const worked = await job.run()
        // Straight back round when there was work: a backlog is drained as
        // fast as the model answers rather than one batch per interval.
        if (!worked) await this.sleep(idleMs)
      } catch (error) {
        logger.error({ err: error }, 'reply worker iteration failed')
        await this.sleep(errorMs)
      }
    }
  }

  private async sleep(ms: number): Promise<void> {
    if (this.stopped) return
    await new Promise<void>((resolve) => {
      const timer = setTimeout(finish, ms)
      this.wake = finish
      function finish(): void {
        clearTimeout(timer)
        resolve()
      }
    })
    this.wake = undefined
  }
}
