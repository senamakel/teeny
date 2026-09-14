import type { FastifyBaseLogger } from 'fastify'
import { applyHouseStyle } from '../agent/house-style.js'
import { ReplyHarness } from '../harness/reply-harness.js'
import type { MentionQueue } from '../queue/mentions.js'
import type { Journal } from '../journal/journal.js'
import type { MediaStore } from '../media/store.js'
import type { SafetyGate } from '../safety/gate.js'
import type { GifSource, ReplyDraft, XClient, XPost, XTools } from '../types.js'

/**
 * Drains the cache. Never talks to X's search.
 *
 * Because the posts are already saved, this half can be slow, can fail, and can
 * be restarted without losing a mention: each attempt is recorded on the queued
 * item, and a failure leaves the post where the next run will find it.
 */
export class ReplyJob {
  private running = false

  constructor(
    private readonly dependencies: {
      x: XClient & Pick<XTools, 'createPost' | 'uploadMedia'>
      harness: ReplyHarness
      queue: MentionQueue
      logger: FastifyBaseLogger
      dryRun: boolean
      maxReplies: number
      gifs?: GifSource
      /** Where the drafter's generate_image / generate_video calls leave their files. */
      media?: MediaStore
      maxEmoji?: number
      /** Last line of defence against answering the account's own posts. */
      botUserId?: () => Promise<string | undefined>
      /**
       * How long to wait between published replies, as a range.
       *
       * Not a rate limit in the API sense -- every call was within X's limits.
       * It is a pacing brake, added after a giveaway had the account answering
       * a hundred people in an hour and X throttled its reach for it: a hundred
       * near-identical posts in quick succession is the shape of spam however
       * sincerely each one is meant.
       *
       * The gap is random within the range rather than fixed, because a fixed
       * one is its own signature. Nothing that types replies by hand produces
       * one exactly every sixty seconds, and a cadence that regular is easier
       * to spot than the volume was.
       *
       * It lives here rather than in the worker on purpose: the worker's batch
       * size would otherwise still let five replies leave back to back.
       */
      minReplyIntervalMs?: number
      /** The top of that range. Defaults to the minimum, i.e. a fixed gap. */
      maxReplyIntervalMs?: number
      /** Injectable for the tests; defaults to Math.random. */
      random?: () => number
      /** The mind, for the landing page. Optional, and never in the way of a reply. */
      journal?: Journal
      /**
       * The content filter, both ways. A mention it holds is never drafted; a
       * draft it holds is never published. Optional only so the tests can
       * build a job without one; the service always passes it.
       */
      safety?: SafetyGate
    },
  ) {}

  /**
   * One line in the journal per mention, whatever happened to it. Failing to
   * write it is logged and nothing else: the reply has already gone out or
   * already failed, and the journal is the record, not the act.
   */
  private async journal(
    post: XPost,
    draft: Partial<ReplyDraft>,
    outcome: 'posted' | 'dry-run' | 'skipped' | 'held' | 'failed',
    note?: string,
  ): Promise<void> {
    const { journal, logger } = this.dependencies
    if (!journal) return
    try {
      await journal.record({
        id: `mention:${post.id}`,
        trigger: 'mention',
        incoming: post.text,
        tone: draft.incomingTone ?? 'neutral',
        recalled: draft.recalled ?? [],
        tools: draft.tools ?? [],
        thinking: draft.thinking ?? '',
        said: outcome === 'posted' || outcome === 'dry-run' ? (draft.text ?? '') : '',
        declined: draft.declined ?? [],
        outcome,
        ...(note ? { outcomeNote: note } : {}),
      })
    } catch (error) {
      logger.warn({ err: error, postId: post.id }, 'could not journal the turn')
    }
  }

  /** When the last reply was actually published, across runs. */
  private lastReplyAt = 0

  /**
   * Holds the next reply back for a randomly chosen gap.
   *
   * A plain sleep, so a shutdown can be up to one gap late. That is the
   * accepted cost of the brake: the alternative is cutting a reply off
   * mid-flight when the container is replaced, which is worse.
   */
  private async pace(): Promise<void> {
    const min = this.dependencies.minReplyIntervalMs ?? 0
    if (min <= 0) return
    const max = Math.max(min, this.dependencies.maxReplyIntervalMs ?? min)
    const random = this.dependencies.random ?? Math.random
    const gap = Math.round(min + random() * (max - min))

    const waited = Date.now() - this.lastReplyAt
    if (this.lastReplyAt > 0 && waited < gap) {
      const remaining = gap - waited
      this.dependencies.logger.info({ gap, remaining }, 'pacing: holding the next reply')
      await new Promise((resolve) => setTimeout(resolve, remaining))
    }
    this.lastReplyAt = Date.now()
  }

  /**
   * Resolves the drafter's GIF query to an uploaded media id. A GIF is a nice
   * touch, never the point of the reply, so any failure here is logged and the
   * reply still goes out as text.
   */
  private async attachGif(query: string): Promise<string[] | undefined> {
    const { gifs, logger } = this.dependencies
    if (!gifs) return undefined
    try {
      // The account's own channel first, because its art is the better answer
      // when it fits. It holds a few dozen GIFs on a narrow set of themes, so
      // most specific queries miss and the rest of Giphy is the fallback
      // rather than the exception.
      const [own] = await gifs.search(query, { channelOnly: true })
      const [gif] = own ? [own] : await gifs.search(query, { channelOnly: false })
      if (!gif) {
        logger.info({ query }, 'no GIF matched the drafted query')
        return undefined
      }
      logger.info({ query, gifId: gif.id, fromChannel: Boolean(own) }, 'attaching a GIF')
      const uploaded = await this.dependencies.x.uploadMedia(await gifs.download(gif))
      return [uploaded.mediaId]
    } catch (error) {
      logger.warn({ err: error, query }, 'GIF attachment failed; replying with text only')
      return undefined
    }
  }

  /**
   * A file the drafter generated, uploaded for attaching. Like a GIF it fails
   * soft: the reply was written to stand on its own, and a picture that did
   * not make it is a smaller loss than a person left unanswered.
   */
  private async attachGenerated(mediaId: string): Promise<string[] | undefined> {
    const { media, logger } = this.dependencies
    const stored = media?.get(mediaId)
    if (!stored) {
      logger.warn({ mediaId }, 'the drafted mediaId is not in the store; replying with text only')
      return undefined
    }
    try {
      const uploaded = await this.dependencies.x.uploadMedia({
        data: stored.data,
        mimeType: stored.mimeType,
      })
      logger.info({ mediaId, kind: stored.kind, bytes: stored.data.byteLength }, 'attaching generated media')
      return [uploaded.mediaId]
    } catch (error) {
      logger.warn({ err: error, mediaId }, 'generated media upload failed; replying with text only')
      return undefined
    }
  }

  /** Resolves true when it answered something, which is what tells a worker
   * loop to come straight back rather than idle. */
  async run(): Promise<boolean> {
    if (this.running) {
      this.dependencies.logger.warn('reply job is already running; skipping overlap')
      return false
    }
    this.running = true
    try {
      const queued = await this.dependencies.queue.takePending(this.dependencies.maxReplies)
      if (queued.length === 0) return false

      // A self-reply is the failure that compounds -- the answer is itself a
      // mention, and answering it produces another -- so not knowing which
      // posts are the account's own stops the run rather than relaxing the
      // check. Nothing is lost: the queue keeps until the next tick.
      const botUserId = await this.dependencies.botUserId?.()
      if (!botUserId) {
        this.dependencies.logger.error(
          'cannot identify the account; refusing to reply to anything this run',
        )
        return false
      }

      for (const item of queued) {
        const { post } = item

        // Every path upstream already excludes the account's own posts. This
        // is the last one standing if any of them regresses.
        if (post.authorId === botUserId) {
          await this.dependencies.queue.markFailed(post.id, 'refused: the account wrote this post')
          this.dependencies.logger.warn(
            { postId: post.id },
            'refused to answer the account\'s own post',
          )
          await this.journal(
            post,
            { thinking: 'This is my own post. Answering it would start a loop of me replying to myself.' },
            'skipped',
            'the account wrote this post',
          )
          continue
        }

        try {
          // The gate, inbound: a post about something the account does not
          // touch is not answered, not remembered, and not shown to the
          // drafter. The journal gets the category and never the text.
          const context = {
            surface: 'x' as const,
            postId: post.id,
            ...(post.conversationId ? { conversationId: post.conversationId } : {}),
            ...(post.authorUsername ? { authorUsername: post.authorUsername } : {}),
          }
          // A mention an operator approved from the review page comes back
          // through the queue with the inbound check waived. The outbound
          // check is not: the approval was of the question, not the answer.
          const incoming = item.filterApproved
            ? undefined
            : await this.dependencies.safety?.inbound(post.text, context)
          if (incoming && !incoming.ok) {
            await this.dependencies.queue.markFailed(post.id, `held: ${incoming.category}`)
            this.dependencies.logger.info(
              { postId: post.id, category: incoming.category, by: incoming.by, reason: incoming.reason },
              'held a mention: the content filter refused it',
            )
            await this.journal(
              { ...post, text: `[held by the content filter: ${incoming.category}]` },
              { thinking: 'This is not something I talk about, so I am not answering it.' },
              'skipped',
              `content filter: ${incoming.category}`,
            )
            continue
          }

          const drafted = await this.dependencies.harness.generate(post)
          const styled = applyHouseStyle(drafted.text, {
            ...(this.dependencies.maxEmoji === undefined
              ? {}
              : { maxEmoji: this.dependencies.maxEmoji }),
            ...(post.authorUsername ? { replyingTo: post.authorUsername } : {}),
          })
          if (styled.fixes.length > 0) {
            this.dependencies.logger.info(
              { postId: post.id, fixes: styled.fixes },
              'house style corrected the draft',
            )
          }

          // The gate, outbound: the styled text is what would be published,
          // so that is what is checked, and what comes back (links cut) is
          // what goes out. A held draft is a mention answered by nobody, which
          // is the right outcome for a draft the account should not have written.
          const outgoing = await this.dependencies.safety?.outbound(styled.text, context)
          if (outgoing) {
            if (outgoing.fixes.length > 0) {
              this.dependencies.logger.warn(
                { postId: post.id, fixes: outgoing.fixes },
                'the content filter edited the draft',
              )
            }
            if (!outgoing.decision.ok) {
              await this.dependencies.queue.markFailed(post.id, `withheld: ${outgoing.decision.category}`)
              this.dependencies.logger.warn(
                { postId: post.id, category: outgoing.decision.category, by: outgoing.decision.by, reason: outgoing.decision.reason },
                'withheld a reply: the content filter refused the draft',
              )
              await this.journal(
                post,
                { ...drafted, text: '' },
                'held',
                `content filter withheld the draft: ${outgoing.decision.category}`,
              )
              continue
            }
            styled.text = outgoing.text
          }

          const mediaIds = this.dependencies.dryRun
            ? undefined
            : drafted.mediaId
              ? await this.attachGenerated(drafted.mediaId)
              : drafted.gifQuery
                ? await this.attachGif(drafted.gifQuery)
                : undefined
          // Immediately before publishing, not before drafting: the model call
          // takes seconds of its own, and pacing from the start of the turn
          // would make the real gap between two posts whatever was left over.
          let posted: { id: string } | undefined
          if (!this.dependencies.dryRun) {
            await this.pace()
            posted = await this.dependencies.x.createPost(styled.text, {
              replyToPostId: post.id,
              ...(mediaIds ? { mediaIds } : {}),
            })
          }

          await this.dependencies.harness.remember(
            post,
            styled.text,
            posted?.id,
            drafted.incomingTone,
          )
          const sour = drafted.incomingTone === 'negative' || drafted.incomingTone === 'hostile'
          if (sour) {
            this.dependencies.logger.info(
              { postId: post.id, tone: drafted.incomingTone },
              'answered but not remembered: the incoming post read as sour',
            )
          }
          await this.dependencies.queue.markReplied(post.id, {
            ...(posted?.id ? { id: posted.id } : {}),
            text: styled.text,
          })
          await this.journal(
            post,
            { ...drafted, text: styled.text },
            this.dependencies.dryRun ? 'dry-run' : 'posted',
            sour ? 'sour tone: answered, not remembered' : undefined,
          )
          this.dependencies.logger.info(
            {
              postId: post.id,
              replyId: posted?.id,
              text: styled.text,
              gifQuery: drafted.gifQuery,
              mediaId: drafted.mediaId,
              mediaAttached: Boolean(mediaIds),
              dryRun: this.dependencies.dryRun,
            },
            'replied to a cached mention',
          )
        } catch (error) {
          const message = error instanceof Error ? error.message : String(error)
          await this.dependencies.queue.markFailed(post.id, message)
          this.dependencies.logger.error(
            { err: error, postId: post.id, attempts: item.attempts + 1 },
            'failed to reply to a cached mention',
          )
          await this.journal(post, {}, 'failed', message)
        }
      }
      // It took a batch, so there may be more behind it.
      return true
    } finally {
      this.running = false
    }
  }
}
