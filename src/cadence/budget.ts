import { mentionsDeliberately, type XPost } from '../types.js'
import type { OwnPost, PostLedger } from './ledger.js'

/**
 * Which budget a reply comes out of.
 *
 * `auto` is the account answering under its own post -- somebody replied to
 * the morning post and the account replies back. `smart` is the reply guy:
 * a mention or a quote out in somebody else's timeline, answered there.
 * They are capped separately because they do different jobs: the first
 * keeps a thread alive under a post that is being seen, the second puts the
 * account in front of people who have not seen it.
 *
 * `called` is somebody typing the handle on purpose, out in the world:
 * "hey @bot, what do you make of this". That is a person asking the account
 * a question to its face, and a question asked to your face is answered
 * whether or not the day's budget is spent. It has no cap. A reply under
 * one of the account's own posts is never `called`, however many times the
 * handle appears in it: that is a comment on the account's post, and it
 * stays in the auto lane with the rest of the thread.
 */
export type Lane = 'auto' | 'smart' | 'called'

export interface Assessment {
  lane: Lane
  /** For the auto lane, the own post the reply would sit under. */
  ownPostId?: string
  allowed: boolean
  used: number
  cap: number
}

export interface ReplyBudgetOptions {
  ledger: PostLedger
  queue: {
    countReplied(filter: { lane: Lane; ownPostId?: string; since?: string }): Promise<number>
  }
  /** Replies the account writes under each of its own posts. */
  autoPerPost: number
  /** Replies out in the world, per own post published in the last day. */
  smartPerPost: number
  /** One slot of the cadence, to count how many a hot post's hold took. */
  gapMs: number
  /** The account's handle, to tell a deliberate mention from an inherited one. */
  botUsername?: string
  /** How long an own post keeps attracting replies worth answering. */
  threadWindowMs?: number
  now?: () => number
}

/**
 * Ten under each post, ten out in the world per post: three posts a day is
 * sixty replies. The held slots of a hot post are not lost, they move: the
 * post the timeline is being kept quiet for gets each held slot's auto and
 * smart allowance added to its own thread, which is where the attention is.
 * A post that names the account on purpose is outside all of this and is
 * always answered.
 */
export class ReplyBudget {
  constructor(private readonly options: ReplyBudgetOptions) {}

  async assess(post: XPost, botUserId: string): Promise<Assessment> {
    const now = (this.options.now ?? Date.now)()
    const window = this.options.threadWindowMs ?? 7 * 24 * 3_600_000
    const own = await this.options.ledger.since(new Date(now - window).toISOString(), 200)

    const under = ownPostOf(post, botUserId, own)
    if (under) {
      const used = await this.options.queue.countReplied({ lane: 'auto', ownPostId: under.postId })
      const cap = this.options.autoPerPost + heldSlots(under, this.options.gapMs) * (this.options.autoPerPost + this.options.smartPerPost)
      return { lane: 'auto', ownPostId: under.postId, allowed: used < cap, used, cap }
    }

    // Somebody typed the handle. Not a reply in the account's own thread,
    // that was handled above; a post out in the world that called the
    // account in by name. Uncapped: `used` is for the record, not the rule.
    if (this.options.botUsername && mentionsDeliberately(post, this.options.botUsername)) {
      const used = await this.options.queue.countReplied({ lane: 'called', since: new Date(now - 24 * 3_600_000).toISOString() })
      return { lane: 'called', allowed: true, used, cap: Number.POSITIVE_INFINITY }
    }

    const dayAgo = new Date(now - 24 * 3_600_000).toISOString()
    const postsToday = own.filter((entry) => entry.postedAt >= dayAgo).length
    const used = await this.options.queue.countReplied({ lane: 'smart', since: dayAgo })
    // A quiet day still gets one post's worth: the reply guy is how a day
    // with nothing to say still talks to somebody.
    const cap = this.options.smartPerPost * Math.max(1, postsToday)
    return { lane: 'smart', allowed: used < cap, used, cap }
  }
}

/**
 * The own post a reply sits under, if it is one. A root post is its own
 * conversation on X, so the conversation id names it directly; a delivery
 * without one that is still a reply to the account is taken to be about the
 * latest post, which is where nearly all of them land.
 */
export function ownPostOf(post: XPost, botUserId: string, own: OwnPost[]): OwnPost | undefined {
  if (post.conversationId) {
    const match = own.find((entry) => entry.postId === post.conversationId)
    if (match) return match
  }
  if (post.inReplyToUserId === botUserId) return own[0]
  return undefined
}

/** How many slots a hot post's hold spans, counted from when it first ran hot. */
export function heldSlots(post: OwnPost, gapMs: number): number {
  if (!post.hot) return 0
  const span = Date.parse(post.hot.holdUntil) - Date.parse(post.hot.since)
  return Math.max(0, Math.ceil(span / gapMs))
}
