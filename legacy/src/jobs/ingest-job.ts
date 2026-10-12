import type { FastifyBaseLogger } from 'fastify'
import type { MentionQueue } from '../queue/mentions.js'
import {
  isWorthAnswering,
  type BotState,
  type MentionWindow,
  type StateStore,
  type XClient,
} from '../types.js'

/**
 * Reads X and writes to the cache. Nothing else.
 *
 * This half is the one with a deadline: X's recent search only reaches back 7
 * days and every post retrieved is billed, so it runs often, asks only for what
 * arrived since the last poll, and hands the result to the queue. Whether a
 * reply is ever written, and how long that takes, is not its problem.
 */
export class IngestJob {
  private running = false

  constructor(
    private readonly dependencies: {
      x: XClient
      queue: MentionQueue
      state: StateStore
      logger: FastifyBaseLogger
      maxResults?: number
      lookbackMinutes?: number
      /** The account's own id, used to tell a reply to it from a reply to a reply. */
      botUserId?: () => Promise<string | undefined>
      /** The handle, used to tell a typed mention from an inherited one. */
      botUsername: string
    },
  ) {}

  /**
   * X v2 rejects a search carrying both `since_id` and `start_time`, so this
   * picks one: the cursor when there is one, a time window from the last poll
   * otherwise, which is what stops a quiet account re-reading 7 days a cycle.
   */
  private window(state: BotState): MentionWindow {
    const lookbackMinutes = this.dependencies.lookbackMinutes ?? 15
    const OVERLAP_MS = 60_000
    const SEVEN_DAYS_MS = 7 * 24 * 60 * 60 * 1_000
    const maxResults = this.dependencies.maxResults ?? 10

    if (state.sinceId) return { sinceId: state.sinceId, maxResults }

    const previous = state.lastPolledAt ? Date.parse(state.lastPolledAt) : Number.NaN
    const from = Number.isNaN(previous)
      ? Date.now() - lookbackMinutes * 60_000
      : previous - OVERLAP_MS

    return {
      startTime: new Date(Math.max(from, Date.now() - SEVEN_DAYS_MS + 60_000)).toISOString(),
      maxResults,
    }
  }

  async run(): Promise<void> {
    if (this.running) {
      this.dependencies.logger.warn('ingest is already running; skipping overlap')
      return
    }
    this.running = true
    try {
      const state = await this.dependencies.state.load()
      const window = this.window(state)
      const polledAt = new Date().toISOString()
      const retrieved = await this.dependencies.x.findMentions(window)

      // Direct replies to the account, plus anything further away where someone
      // actually typed the handle. What is dropped is the thread the account was
      // tagged into once and then carried along in every reply after.
      const botUserId = await this.dependencies.botUserId?.()
      const shallow = retrieved.filter(
        (post) =>
          !(botUserId && post.authorId === botUserId) &&
          isWorthAnswering(post, {
            ...(botUserId ? { botUserId } : {}),
            botUsername: this.dependencies.botUsername,
          }),
      )
      const cached = await this.dependencies.queue.cache(shallow)

      // The cursor advances past everything cached, not past everything
      // replied to, because the queue owns the reply work from here.
      const newest = retrieved.reduce<string | undefined>(
        (highest, post) =>
          !highest || BigInt(post.id) > BigInt(highest) ? post.id : highest,
        state.sinceId,
      )
      await this.dependencies.state.save({
        ...state,
        lastPolledAt: polledAt,
        ...(newest ? { sinceId: newest } : {}),
      })

      this.dependencies.logger.info(
        {
          retrieved: retrieved.length,
          tooDeep: retrieved.length - shallow.length,
          cached,
          ...window,
        },
        'ingested X mentions',
      )
    } finally {
      this.running = false
    }
  }
}
