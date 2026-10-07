import { MongoClient, type Collection } from 'mongodb'

/** One reading of a post's counters, taken by the heat job. */
export interface MetricSample {
  at: string
  impressions: number
  likes: number
  replies: number
  reposts: number
  quotes: number
}

/**
 * A post the account published to its own timeline: not a reply, which is
 * somebody else's conversation, but a morning post, a chart, a moon chart,
 * a calendar item, or something the agent was asked to post.
 */
export interface OwnPost {
  postId: string
  /** Which job published it: morning, chart, moon, calendar:<name>, agent, post. */
  kind: string
  postedAt: string
  text?: string
  samples: MetricSample[]
  /**
   * Set when the heat job saw the post take off. `holdUntil` is how long the
   * timeline stays quiet for it; the reply budget reads the same field.
   */
  hot?: { since: string; holdUntil: string }
}

export interface PostLedger {
  record(post: { postId: string; kind: string; postedAt: string; text?: string }): Promise<void>
  /** The most recent own post, by when it was posted. */
  latest(): Promise<OwnPost | undefined>
  /** Own posts published at or after `since`, newest first. */
  since(since: string, limit?: number): Promise<OwnPost[]>
  get(postId: string): Promise<OwnPost | undefined>
  sample(postId: string, sample: MetricSample): Promise<void>
  setHot(postId: string, hot: { since: string; holdUntil: string }): Promise<void>
  /** The hot post whose hold is still running at `now`, if any. */
  hold(now: string): Promise<OwnPost | undefined>
  close(): Promise<void>
}

/**
 * The record of what the account put on its timeline and when.
 *
 * The posting jobs each guard themselves against running twice, but none of
 * them knew about the others: a morning post at seven and a chart at eight
 * were two well-behaved jobs and one badly paced account. This is the one
 * place they all write, so the cadence can read a single answer to "when
 * did we last post", and the heat job has the ids to watch.
 */
export class MongoPostLedger implements PostLedger {
  private readonly client: MongoClient
  private readonly posts: Collection<OwnPost>
  private indexed = false

  constructor(options: { uri: string; database: string; collection?: string }) {
    this.client = new MongoClient(options.uri)
    this.posts = this.client.db(options.database).collection<OwnPost>(options.collection ?? 'own_posts')
  }

  private async ready(): Promise<void> {
    if (this.indexed) return
    await this.client.connect()
    await this.posts.createIndex({ postId: 1 }, { unique: true })
    await this.posts.createIndex({ postedAt: -1 })
    await this.posts.createIndex({ 'hot.holdUntil': -1 })
    this.indexed = true
  }

  async record(post: { postId: string; kind: string; postedAt: string; text?: string }): Promise<void> {
    await this.ready()
    await this.posts.updateOne(
      { postId: post.postId },
      { $setOnInsert: { ...post, samples: [] } },
      { upsert: true },
    )
  }

  async latest(): Promise<OwnPost | undefined> {
    await this.ready()
    return (await this.posts.find().sort({ postedAt: -1 }).limit(1).next()) ?? undefined
  }

  async since(since: string, limit = 50): Promise<OwnPost[]> {
    await this.ready()
    return this.posts.find({ postedAt: { $gte: since } }).sort({ postedAt: -1 }).limit(limit).toArray()
  }

  async get(postId: string): Promise<OwnPost | undefined> {
    await this.ready()
    return (await this.posts.findOne({ postId })) ?? undefined
  }

  async sample(postId: string, sample: MetricSample): Promise<void> {
    await this.ready()
    // The last hundred readings are plenty for a velocity; a post is watched
    // for a day at most and the job runs a few times an hour.
    await this.posts.updateOne({ postId }, { $push: { samples: { $each: [sample], $slice: -100 } } })
  }

  async setHot(postId: string, hot: { since: string; holdUntil: string }): Promise<void> {
    await this.ready()
    await this.posts.updateOne({ postId }, { $set: { hot } })
  }

  async hold(now: string): Promise<OwnPost | undefined> {
    await this.ready()
    return (
      (await this.posts.find({ 'hot.holdUntil': { $gt: now } }).sort({ 'hot.holdUntil': -1 }).limit(1).next()) ??
      undefined
    )
  }

  async close(): Promise<void> {
    await this.client.close()
  }
}

/** The same ledger in memory, for the tests and the dry-run scripts. */
export class InMemoryPostLedger implements PostLedger {
  readonly posts = new Map<string, OwnPost>()

  async record(post: { postId: string; kind: string; postedAt: string; text?: string }): Promise<void> {
    if (!this.posts.has(post.postId)) this.posts.set(post.postId, { ...post, samples: [] })
  }

  private sorted(): OwnPost[] {
    return [...this.posts.values()].sort((a, b) => b.postedAt.localeCompare(a.postedAt))
  }

  async latest(): Promise<OwnPost | undefined> {
    return this.sorted()[0]
  }

  async since(since: string, limit = 50): Promise<OwnPost[]> {
    return this.sorted()
      .filter((post) => post.postedAt >= since)
      .slice(0, limit)
  }

  async get(postId: string): Promise<OwnPost | undefined> {
    return this.posts.get(postId)
  }

  async sample(postId: string, sample: MetricSample): Promise<void> {
    const post = this.posts.get(postId)
    if (post) post.samples = [...post.samples, sample].slice(-100)
  }

  async setHot(postId: string, hot: { since: string; holdUntil: string }): Promise<void> {
    const post = this.posts.get(postId)
    if (post) post.hot = hot
  }

  async hold(now: string): Promise<OwnPost | undefined> {
    return [...this.posts.values()]
      .filter((post) => post.hot && post.hot.holdUntil > now)
      .sort((a, b) => b.hot!.holdUntil.localeCompare(a.hot!.holdUntil))[0]
  }

  async close(): Promise<void> {}
}
