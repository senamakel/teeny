import { MongoClient, type Collection } from 'mongodb'

/** What somebody did that involved the account. */
export type Interaction = 'mention' | 'reply' | 'quote' | 'repost' | 'like' | 'follow' | 'answered'

/**
 * How much each interaction is worth when the account decides whom to
 * answer first. Writing something is worth more than a tap; a quote, which
 * puts the account in front of the writer's followers, most of all. Being
 * answered counts too, a little: the people the account talks with are the
 * people it should keep talking with.
 */
export const WEIGHTS: Record<Interaction, number> = {
  quote: 4,
  reply: 3,
  mention: 3,
  repost: 2,
  follow: 2,
  like: 1,
  answered: 1,
}

export interface PersonRecord {
  userId: string
  username?: string
  score: number
  counts: Partial<Record<Interaction, number>>
  firstSeenAt: string
  lastSeenAt: string
}

export interface People {
  observe(event: { userId: string; username?: string; kind: Interaction; at?: string }): Promise<void>
  /** Scores for the ids given; an unknown person scores 0. */
  scores(userIds: string[]): Promise<Map<string, number>>
  top(limit: number): Promise<PersonRecord[]>
  /** The person last seen under this handle, ignoring case and a leading @. */
  byUsername?(username: string): Promise<PersonRecord | undefined>
  close(): Promise<void>
}

/**
 * Who interacts with the account, and how much.
 *
 * The reply budget is small on purpose, so the question of who gets one of
 * the ten answers under a post matters. This is the memory that decides it:
 * every like, follow, reply and quote that reaches the webhook adds to the
 * person's score, and the reply job answers the highest scores first. The
 * score is a sum rather than a rate, so a regular who has been around for a
 * month outranks a stranger who liked three things this morning, which is
 * the intended bias.
 */
export class MongoPeople implements People {
  private readonly client: MongoClient
  private readonly people: Collection<PersonRecord>
  private indexed = false

  constructor(options: { uri: string; database: string; collection?: string }) {
    this.client = new MongoClient(options.uri)
    this.people = this.client.db(options.database).collection<PersonRecord>(options.collection ?? 'people')
  }

  private async ready(): Promise<void> {
    if (this.indexed) return
    await this.client.connect()
    await this.people.createIndex({ userId: 1 }, { unique: true })
    await this.people.createIndex({ score: -1 })
    await this.people.createIndex({ username: 1 })
    this.indexed = true
  }

  async observe(event: { userId: string; username?: string; kind: Interaction; at?: string }): Promise<void> {
    await this.ready()
    const at = event.at ?? new Date().toISOString()
    await this.people.updateOne(
      { userId: event.userId },
      {
        $inc: { score: WEIGHTS[event.kind], [`counts.${event.kind}`]: 1 },
        $set: { lastSeenAt: at, ...(event.username ? { username: event.username } : {}) },
        $setOnInsert: { userId: event.userId, firstSeenAt: at },
      },
      { upsert: true },
    )
  }

  async scores(userIds: string[]): Promise<Map<string, number>> {
    if (userIds.length === 0) return new Map()
    await this.ready()
    const rows = await this.people.find({ userId: { $in: userIds } }, { projection: { userId: 1, score: 1 } }).toArray()
    return new Map(rows.map((row) => [row.userId, row.score]))
  }

  async top(limit: number): Promise<PersonRecord[]> {
    await this.ready()
    return this.people.find().sort({ score: -1 }).limit(limit).toArray()
  }

  async byUsername(username: string): Promise<PersonRecord | undefined> {
    await this.ready()
    const handle = username.replace(/^@/, '')
    if (!/^\w{1,15}$/.test(handle)) return undefined
    const row = await this.people
      .find({ username: { $regex: `^${handle}$`, $options: 'i' } })
      .sort({ lastSeenAt: -1 })
      .limit(1)
      .next()
    return row ?? undefined
  }

  async close(): Promise<void> {
    await this.client.close()
  }
}

export class InMemoryPeople implements People {
  readonly people = new Map<string, PersonRecord>()

  async observe(event: { userId: string; username?: string; kind: Interaction; at?: string }): Promise<void> {
    const at = event.at ?? new Date().toISOString()
    const existing = this.people.get(event.userId) ?? {
      userId: event.userId,
      score: 0,
      counts: {},
      firstSeenAt: at,
      lastSeenAt: at,
    }
    existing.score += WEIGHTS[event.kind]
    existing.counts[event.kind] = (existing.counts[event.kind] ?? 0) + 1
    existing.lastSeenAt = at
    if (event.username) existing.username = event.username
    this.people.set(event.userId, existing)
  }

  async scores(userIds: string[]): Promise<Map<string, number>> {
    return new Map(userIds.map((id) => [id, this.people.get(id)?.score ?? 0]))
  }

  async top(limit: number): Promise<PersonRecord[]> {
    return [...this.people.values()].sort((a, b) => b.score - a.score).slice(0, limit)
  }

  async byUsername(username: string): Promise<PersonRecord | undefined> {
    const handle = username.replace(/^@/, '').toLowerCase()
    return [...this.people.values()]
      .filter((person) => person.username?.toLowerCase() === handle)
      .sort((a, b) => b.lastSeenAt.localeCompare(a.lastSeenAt))[0]
  }

  async close(): Promise<void> {}
}
