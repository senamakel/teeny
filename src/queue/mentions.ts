import { MongoClient, type Collection } from 'mongodb'
import type { XPost } from '../types.js'

export type MentionStatus = 'pending' | 'replied' | 'failed' | 'skipped'

export interface QueuedMention {
  postId: string
  post: XPost
  status: MentionStatus
  attempts: number
  cachedAt: string
  updatedAt: string
  replyId?: string
  replyText?: string
  error?: string
}

export interface MentionQueue {
  /** Caches posts that are not already known. Returns how many were new. */
  cache(posts: XPost[]): Promise<number>
  /** Oldest pending posts first, so a backlog drains in the order it arrived. */
  takePending(limit: number): Promise<QueuedMention[]>
  markReplied(postId: string, reply: { id?: string; text: string }): Promise<void>
  markFailed(postId: string, error: string): Promise<void>
  counts(): Promise<Record<MentionStatus, number>>
  close(): Promise<void>
}

/**
 * The cache that separates reading X from replying to it.
 *
 * Retrieval is the expensive, rate-limited half and it is the half that cannot
 * be retried later: a post outside the 7-day search window is simply gone. So
 * ingest writes here and returns, and replying drains this at whatever pace the
 * model and the posting limits allow. A crash between the two loses nothing.
 */
export class MongoMentionQueue implements MentionQueue {
  private readonly client: MongoClient
  private readonly mentions: Collection<QueuedMention>
  private indexed = false

  constructor(options: { uri: string; database: string; collection?: string }) {
    this.client = new MongoClient(options.uri)
    this.mentions = this.client
      .db(options.database)
      .collection<QueuedMention>(options.collection ?? 'mentions')
  }

  private async ready(): Promise<void> {
    if (this.indexed) return
    await this.client.connect()
    await this.mentions.createIndex({ postId: 1 }, { unique: true })
    await this.mentions.createIndex({ status: 1, cachedAt: 1 })
    this.indexed = true
  }

  async cache(posts: XPost[]): Promise<number> {
    if (posts.length === 0) return 0
    await this.ready()
    const now = new Date().toISOString()

    // Upsert-if-absent: a post already in the queue keeps its status, so a
    // re-read from the overlapping search window never resets work.
    const result = await this.mentions.bulkWrite(
      posts.map((post) => ({
        updateOne: {
          filter: { postId: post.id },
          update: {
            $setOnInsert: {
              postId: post.id,
              post,
              status: 'pending' as const,
              attempts: 0,
              cachedAt: now,
              updatedAt: now,
            },
          },
          upsert: true,
        },
      })),
      { ordered: false },
    )
    return result.upsertedCount
  }

  async takePending(limit: number): Promise<QueuedMention[]> {
    await this.ready()
    return this.mentions
      .find({ status: 'pending' })
      .sort({ cachedAt: 1 })
      .limit(limit)
      .toArray()
  }

  async markReplied(postId: string, reply: { id?: string; text: string }): Promise<void> {
    await this.ready()
    await this.mentions.updateOne(
      { postId },
      {
        $set: {
          status: 'replied',
          updatedAt: new Date().toISOString(),
          replyText: reply.text,
          ...(reply.id ? { replyId: reply.id } : {}),
        },
        $inc: { attempts: 1 },
      },
    )
  }

  async markFailed(postId: string, error: string): Promise<void> {
    await this.ready()
    await this.mentions.updateOne(
      { postId },
      {
        $set: { status: 'failed', error: error.slice(0, 500), updatedAt: new Date().toISOString() },
        $inc: { attempts: 1 },
      },
    )
  }

  async counts(): Promise<Record<MentionStatus, number>> {
    await this.ready()
    const rows = await this.mentions
      .aggregate<{ _id: MentionStatus; total: number }>([
        { $group: { _id: '$status', total: { $sum: 1 } } },
      ])
      .toArray()
    const counts: Record<MentionStatus, number> = {
      pending: 0,
      replied: 0,
      failed: 0,
      skipped: 0,
    }
    for (const row of rows) counts[row._id] = row.total
    return counts
  }

  async close(): Promise<void> {
    await this.client.close()
  }
}
