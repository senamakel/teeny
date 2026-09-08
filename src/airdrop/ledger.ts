import { MongoClient, type Collection } from 'mongodb'

/**
 * Every airdrop the account has ever sent, and the only source of truth for the
 * budget and the once-per-account rule.
 *
 * It is written twice per airdrop on purpose: a `sending` row is claimed BEFORE
 * the transfer is broadcast, and updated to `sent` with the hash afterwards. A
 * crash between the two leaves the row behind, which is the safe way round --
 * an airdrop that may not have happened blocks a retry, where the opposite
 * would pay twice for one post. The unique index on postId is what makes the
 * claim atomic, so two concurrent turns answering the same post cannot both
 * proceed.
 */
export interface AirdropRecord {
  postId: string
  to: string
  amount: number
  recipientUserId?: string
  recipientUsername?: string
  status: 'sending' | 'sent' | 'failed'
  txHash?: string
  error?: string
  createdAt: string
  updatedAt: string
}

export interface AirdropLedger {
  /** Tokens sent in the trailing 24 hours, counting in-flight claims. */
  sentToday(): Promise<number>
  /** How many times this X account has been paid. */
  paidRecipientCount(recipientUserId?: string): Promise<number>
  /** An existing airdrop for this post, whatever its outcome. */
  forPost(postId: string): Promise<AirdropRecord | undefined>
  /** Reserve the post. Returns false when somebody already has it. */
  claim(record: Omit<AirdropRecord, 'status' | 'createdAt' | 'updatedAt'>): Promise<boolean>
  settle(postId: string, txHash: string): Promise<void>
  fail(postId: string, error: string): Promise<void>
  close(): Promise<void>
}

export class MongoAirdropLedger implements AirdropLedger {
  private readonly client: MongoClient
  private readonly airdrops: Collection<AirdropRecord>
  private indexed?: Promise<void>

  constructor(options: { uri: string; database: string; collection?: string }) {
    this.client = new MongoClient(options.uri)
    this.airdrops = this.client
      .db(options.database)
      .collection<AirdropRecord>(options.collection ?? 'airdrops')
  }

  private async ready(): Promise<void> {
    this.indexed ??= (async () => {
      await this.client.connect()
      // One airdrop per post, enforced by the database rather than by a check
      // the model or a race could get past.
      await this.airdrops.createIndex({ postId: 1 }, { unique: true })
      await this.airdrops.createIndex({ createdAt: -1 })
      await this.airdrops.createIndex({ recipientUserId: 1 })
    })()
    await this.indexed
  }

  async sentToday(): Promise<number> {
    await this.ready()
    const since = new Date(Date.now() - 24 * 60 * 60 * 1000).toISOString()
    const [total] = await this.airdrops
      .aggregate<{ total: number }>([
        // `sending` counts too: an in-flight transfer is money leaving, and a
        // budget that only counts confirmed sends can be spent twice over
        // while the first one is still in the mempool.
        { $match: { createdAt: { $gte: since }, status: { $in: ['sending', 'sent'] } } },
        { $group: { _id: null, total: { $sum: '$amount' } } },
      ])
      .toArray()
    return total?.total ?? 0
  }

  async paidRecipientCount(recipientUserId?: string): Promise<number> {
    if (!recipientUserId) return 0
    await this.ready()
    return this.airdrops.countDocuments({
      recipientUserId,
      status: { $in: ['sending', 'sent'] },
    })
  }

  async forPost(postId: string): Promise<AirdropRecord | undefined> {
    await this.ready()
    return (await this.airdrops.findOne({ postId })) ?? undefined
  }

  async claim(
    record: Omit<AirdropRecord, 'status' | 'createdAt' | 'updatedAt'>,
  ): Promise<boolean> {
    await this.ready()
    const now = new Date().toISOString()
    try {
      await this.airdrops.insertOne({
        ...record,
        status: 'sending',
        createdAt: now,
        updatedAt: now,
      } as AirdropRecord)
      return true
    } catch (error) {
      // Duplicate key: somebody else already claimed this post.
      if ((error as { code?: number }).code === 11000) return false
      throw error
    }
  }

  async settle(postId: string, txHash: string): Promise<void> {
    await this.ready()
    await this.airdrops.updateOne(
      { postId },
      { $set: { status: 'sent', txHash, updatedAt: new Date().toISOString() } },
    )
  }

  async fail(postId: string, error: string): Promise<void> {
    await this.ready()
    await this.airdrops.updateOne(
      { postId },
      { $set: { status: 'failed', error: error.slice(0, 500), updatedAt: new Date().toISOString() } },
    )
  }

  async close(): Promise<void> {
    await this.client.close()
  }
}
