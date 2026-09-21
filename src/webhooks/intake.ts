import type { FastifyBaseLogger } from 'fastify'
import type { MentionQueue } from '../queue/mentions.js'
import type { Memory } from '../types.js'
import type { People } from '../cadence/people.js'
import {
  activityEvent,
  captureWebhookActivity,
  interactionsFromActivity,
  mentionsFromActivity,
} from './activity.js'

/** The four levels these use; narrower than Fastify's so a closure over a logger not yet built will do. */
export type ActivityLogger = Pick<FastifyBaseLogger, 'debug' | 'info' | 'warn' | 'error'>

/**
 * One delivery of the account's activity, whichever way it arrived.
 *
 * X has two transports for the same subscription: the webhook, and the
 * persistent `GET /2/activity/stream`. Since 2026-08 the webhook only carries
 * the free events (likes, follows) and silently drops every `post.*` event,
 * while the stream carries all of them within seconds -- a known X-side fault
 * (devcommunity 273644) with no developer-side fix. So the process reads both,
 * and everything either one delivers comes through here: identity first, the
 * people record, the mention queue, then memory.
 *
 * The same like arrives on both transports, and `people.observe` counts what
 * it is given, so deliveries are deduplicated by X's own event id before any
 * of that runs. The set is bounded and in-process, which is enough: the two
 * transports run in one process, and a duplicate that slips past a restart
 * costs a like on somebody's score, not a second reply.
 */
export class ActivityIntake {
  private readonly seen = new Set<string>()

  constructor(
    private readonly dependencies: {
      memory: Memory
      logger: ActivityLogger
      queue?: MentionQueue
      /** The account's own id, so its own posts are not answered. */
      botUserId?: () => Promise<string | undefined>
      botUsername?: string
      people?: Pick<People, 'observe'>
      /** How many event ids to remember for deduplication. */
      remember?: number
    },
  ) {}

  /**
   * Whether the delivery is one this process has already handled. Only the
   * X Activity API envelope carries an event id; the old shape is never
   * duplicated because only the webhook speaks it.
   */
  private duplicate(payload: Record<string, unknown>): boolean {
    const event = activityEvent(payload)
    if (!event?.eventId) return false
    const key = `${event.eventType}:${event.eventId}`
    if (this.seen.has(key)) return true
    this.seen.add(key)
    const limit = this.dependencies.remember ?? 10_000
    if (this.seen.size > limit) {
      const oldest = this.seen.values().next().value
      if (oldest !== undefined) this.seen.delete(oldest)
    }
    return false
  }

  /**
   * Handles one delivery. Resolves once the queue has it; the memory write is
   * not waited for, because neither transport can afford to: X drops a
   * webhook it does not see acknowledged in seconds, and a stream reader that
   * stalls on one event backs up every event behind it.
   */
  async deliver(
    payload: Record<string, unknown>,
    source: 'webhook' | 'stream',
  ): Promise<{ queued: number; duplicate: boolean }> {
    const { logger, queue, people, memory } = this.dependencies
    if (this.duplicate(payload)) {
      logger.debug({ source }, 'dropped a delivery already handled on the other transport')
      return { queued: 0, duplicate: true }
    }

    let queued = 0
    if (queue) {
      // Identity first, and nothing is queued without it: a delivery is mostly
      // the account's own activity, and queuing that is how a self-reply loop
      // starts. The cron poll is the safety net for anything dropped here.
      let botUserId: string | undefined
      try {
        botUserId = await this.dependencies.botUserId?.()
      } catch (error) {
        logger.error({ err: error, source }, 'could not identify the account; queued nothing')
      }
      if (!botUserId) {
        logger.error({ source }, 'no account id available; refusing to queue activity')
        return { queued: 0, duplicate: false }
      }
      if (people) {
        const interactions = interactionsFromActivity(payload, botUserId)
        await Promise.all(
          interactions.map((interaction) =>
            people.observe(interaction).catch((error: unknown) => {
              logger.warn({ err: error, interaction, source }, 'could not record the interaction')
            }),
          ),
        )
      }
      const mentions = mentionsFromActivity(payload, {
        botUserId,
        botUsername: this.dependencies.botUsername ?? '',
      })
      if (mentions.length > 0) {
        queued = await queue.cache(mentions)
        logger.info({ delivered: mentions.length, cached: queued, source }, 'queued mentions from activity')
      }
    }

    void captureWebhookActivity(memory, payload).catch((error: unknown) => {
      logger.warn({ err: error, source }, 'could not capture the activity in memory')
    })
    return { queued, duplicate: false }
  }
}
