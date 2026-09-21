import { AsyncLocalStorage } from 'node:async_hooks'
import type { XArticleDraft, XCreateOptions, XTools } from '../types.js'
import type { PostLedger } from './ledger.js'

export type Claim =
  | { ok: true }
  | { ok: false; reason: 'gap'; lastPostId: string; lastPostedAt: string; until: string }
  | { ok: false; reason: 'hot'; hotPostId: string; until: string }

export interface CadenceOptions {
  ledger: PostLedger
  /** The least time between two of the account's own posts. */
  minGapMs: number
  /**
   * How much short of the gap still counts as the gap. The jobs fire on the
   * hour and then spend minutes drawing and filming before they post, so a
   * schedule of exactly eight hours apart lands at seven hours fifty; this
   * is what keeps that from reading as a violation.
   */
  toleranceMs?: number
  now?: () => number
  logger?: { info: (object: unknown, message?: string) => void; warn: (object: unknown, message?: string) => void }
}

/**
 * The content calendar's one rule: the account's own posts are spaced out,
 * and a post that is taking off gets the timeline to itself.
 *
 * Every job that posts to the timeline asks here first, and the posting
 * client is wrapped so that a post nobody asked about is refused anyway.
 * Two reasons to say no:
 *
 * - the gap: the last own post was less than `minGapMs` ago. Three posts a
 *   day, eight hours apart, is the cadence; two an hour apart is the
 *   repetitive-publishing shape the ranker demotes, and the jobs cannot see
 *   each other's timers, so this is where the rule lives.
 * - the hold: the heat job saw a post's impressions climbing and put a hold
 *   on the slots after it. Posting into that is competing with yourself: the
 *   ranker's author-diversity rule shows a viewer one of the two, and the new
 *   one has no engagement yet. The reply budget spends that slot on the hot
 *   post's thread instead.
 *
 * Replies are never subject to this; they are conversations, not slots.
 */
export class Cadence {
  private readonly kinds = new AsyncLocalStorage<string>()

  constructor(private readonly options: CadenceOptions) {}

  async claim(kind: string): Promise<Claim> {
    const now = (this.options.now ?? Date.now)()
    const nowIso = new Date(now).toISOString()
    const tolerance = this.options.toleranceMs ?? 0

    const hot = await this.options.ledger.hold(nowIso)
    if (hot?.hot) {
      const claim: Claim = { ok: false, reason: 'hot', hotPostId: hot.postId, until: hot.hot.holdUntil }
      this.options.logger?.info({ kind, ...claim }, 'cadence: the timeline is held for a hot post')
      return claim
    }

    const last = await this.options.ledger.latest()
    if (last) {
      const since = now - Date.parse(last.postedAt)
      if (since + tolerance < this.options.minGapMs) {
        const claim: Claim = {
          ok: false,
          reason: 'gap',
          lastPostId: last.postId,
          lastPostedAt: last.postedAt,
          until: new Date(Date.parse(last.postedAt) + this.options.minGapMs).toISOString(),
        }
        this.options.logger?.info({ kind, sinceMs: since, ...claim }, 'cadence: too soon after the last post')
        return claim
      }
    }
    return { ok: true }
  }

  async record(post: { postId: string; kind: string; text?: string }): Promise<void> {
    const now = (this.options.now ?? Date.now)()
    await this.options.ledger.record({ ...post, postedAt: new Date(now).toISOString() })
  }

  /** Runs `fn` with every own post it publishes recorded under `kind`. */
  within<T>(kind: string, fn: () => Promise<T>): Promise<T> {
    return this.kinds.run(kind, fn)
  }

  /** The kind of the job running now, when one said. */
  kindNow(): string | undefined {
    return this.kinds.getStore()
  }
}

export class CadenceHeld extends Error {
  constructor(readonly claim: Exclude<Claim, { ok: true }>) {
    super(
      claim.reason === 'gap'
        ? `cadence: too soon after post ${claim.lastPostId} (${claim.lastPostedAt}); next slot at ${claim.until}`
        : `cadence: the timeline is held for hot post ${claim.hotPostId} until ${claim.until}`,
    )
    this.name = 'CadenceHeld'
  }
}

/**
 * The posting client with the cadence in front of it. An own post -- one
 * that is not a reply -- is claimed before it goes out and recorded after;
 * a reply passes straight through. This is the backstop under the jobs'
 * own checks, and the only check an ad-hoc agent task gets.
 */
export function paced<T extends Pick<XTools, 'createPost'> & Partial<Pick<XTools, 'publishArticle'>>>(x: T, cadence: Cadence): T {
  const createPost = async (text: string, options: XCreateOptions = {}): Promise<{ id: string }> => {
    if (options.replyToPostId) return x.createPost(text, options)
    const kind = cadence.kindNow() ?? 'post'
    const claim = await cadence.claim(kind)
    if (!claim.ok) throw new CadenceHeld(claim)
    const posted = await x.createPost(text, options)
    await cadence.record({ postId: posted.id, kind, text })
    return posted
  }
  // An article is an own post too: publishing puts one on the timeline.
  const publishArticle = async (draft: XArticleDraft): Promise<{ articleId: string; postId: string }> => {
    if (!x.publishArticle) throw new Error('this client cannot publish articles')
    const kind = cadence.kindNow() ?? 'article'
    const claim = await cadence.claim(kind)
    if (!claim.ok) throw new CadenceHeld(claim)
    const published = await x.publishArticle(draft)
    await cadence.record({ postId: published.postId, kind, text: draft.title })
    return published
  }
  return new Proxy(x, {
    get(target, property, receiver) {
      if (property === 'createPost') return createPost
      if (property === 'publishArticle' && typeof target.publishArticle === 'function') return publishArticle
      const value = Reflect.get(target, property, receiver) as unknown
      return typeof value === 'function' ? (value as (...args: unknown[]) => unknown).bind(target) : value
    },
  })
}
