import type { FastifyBaseLogger } from 'fastify'
import { applyHouseStyle } from '../agent/house-style.js'
import { ReplyHarness } from '../harness/reply-harness.js'
import type { MentionQueue } from '../queue/mentions.js'
import type { GifSource, XClient, XTools } from '../types.js'

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
      maxEmoji?: number
      /** Last line of defence against answering the account's own posts. */
      botUserId?: () => Promise<string | undefined>
    },
  ) {}

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
          continue
        }

        try {
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

          const mediaIds =
            this.dependencies.dryRun || !drafted.gifQuery
              ? undefined
              : await this.attachGif(drafted.gifQuery)
          const posted = this.dependencies.dryRun
            ? undefined
            : await this.dependencies.x.createPost(styled.text, {
                replyToPostId: post.id,
                ...(mediaIds ? { mediaIds } : {}),
              })

          await this.dependencies.harness.remember(
            post,
            styled.text,
            posted?.id,
            drafted.incomingTone,
          )
          if (drafted.incomingTone === 'negative' || drafted.incomingTone === 'hostile') {
            this.dependencies.logger.info(
              { postId: post.id, tone: drafted.incomingTone },
              'answered but not remembered: the incoming post read as sour',
            )
          }
          await this.dependencies.queue.markReplied(post.id, {
            ...(posted?.id ? { id: posted.id } : {}),
            text: styled.text,
          })
          this.dependencies.logger.info(
            {
              postId: post.id,
              replyId: posted?.id,
              text: styled.text,
              gifQuery: drafted.gifQuery,
              gifAttached: Boolean(mediaIds),
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
        }
      }
      // It took a batch, so there may be more behind it.
      return true
    } finally {
      this.running = false
    }
  }
}
