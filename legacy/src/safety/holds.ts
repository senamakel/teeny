/**
 * The record of everything the content filter stopped, kept for a person to
 * look at afterwards.
 *
 * The filter is deliberately blunt, and a blunt filter is wrong sometimes.
 * The journal on the public page says only that something was held; this
 * log, which never leaves the box, keeps the text, so an operator can read
 * what was actually refused and say whether the filter was right. That
 * answer is not only a note: approving a held link puts its host on the live
 * allowlist, approving a held mention sends it back to the queue to be
 * answered, and approving a withheld draft releases it. Rejecting confirms
 * the filter and closes the entry.
 *
 * Every write here is best-effort from the gate's point of view. A log that
 * cannot be reached must never turn a hold into a publish, and must never
 * take a reply down.
 */
import { MongoClient, type Collection } from 'mongodb'
import type { Direction } from './screen.js'

export type HoldStatus = 'pending' | 'approved' | 'rejected'

/**
 * What was held. `message` is somebody else's text on the way in; `draft` is
 * the account's own on the way out; `link` is a draft that went out with its
 * links cut, so the cut links are what is under review; `prompt` is a picture
 * the account declined to draw; `task` is an operator's ad-hoc task.
 */
export type HoldKind = 'message' | 'draft' | 'link' | 'prompt' | 'task'

export type HoldSurface = 'x' | 'telegram' | 'task' | 'chart' | 'calendar' | 'morning' | 'moon' | 'narrative' | 'article' | 'quote' | 'lore'

/** Where the held text came from, enough to act on an approval. */
export interface HoldContext {
  surface: HoldSurface
  kind?: HoldKind
  postId?: string
  conversationId?: string
  authorUsername?: string
  chatId?: string
  messageId?: string
  messageThreadId?: string
  authorName?: string
  tool?: string
}

export interface Hold {
  id: string
  at: string
  direction: Direction
  kind: HoldKind
  context: HoldContext
  /** The lexicon category, the judge's category, `links`, or `unknown`. */
  category: string
  reason: string
  by: 'screen' | 'judge' | 'links'
  /** The held text itself. Private: this collection is never served publicly. */
  text: string
  /** For a `link` hold, what was cut. */
  links?: Array<{ link: string; reason: string }>
  status: HoldStatus
  decidedAt?: string
  note?: string
  /** What the approval did, for the dashboard: "requeued", "allowlisted x.example", "released as 123". */
  effect?: string
}

export type HoldInput = Omit<Hold, 'id' | 'at' | 'status' | 'decidedAt' | 'note' | 'effect'>

export interface HoldLog {
  record(entry: HoldInput): Promise<Hold>
  list(options: { status?: HoldStatus; limit?: number }): Promise<Hold[]>
  get(id: string): Promise<Hold | undefined>
  decide(id: string, decision: 'approved' | 'rejected', detail?: { note?: string; effect?: string }): Promise<Hold | undefined>
  counts(): Promise<Record<HoldStatus, number>>
  /**
   * Hosts an operator approved, in allowlist spelling. The gate reads these
   * alongside the built-in list, so an approval takes effect everywhere the
   * gate runs, not only in the process that took it.
   */
  allowedLinks(): Promise<string[]>
  allowLink(entry: string, detail: { holdId: string }): Promise<void>
}

function complete(entry: HoldInput): Hold {
  return {
    ...entry,
    id: `h-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`,
    at: new Date().toISOString(),
    status: 'pending',
  }
}

interface AllowedLink {
  entry: string
  holdId: string
  at: string
}

export class MongoHoldLog implements HoldLog {
  private readonly client: MongoClient
  private readonly holds: Collection<Hold>
  private readonly links: Collection<AllowedLink>
  private indexed = false

  constructor(options: { uri: string; database: string; collection?: string }) {
    this.client = new MongoClient(options.uri)
    const db = this.client.db(options.database)
    const name = options.collection ?? 'holds'
    this.holds = db.collection<Hold>(name)
    this.links = db.collection<AllowedLink>(`${name}_links`)
  }

  private async ready(): Promise<void> {
    if (this.indexed) return
    await this.client.connect()
    await this.holds.createIndex({ id: 1 }, { unique: true })
    await this.holds.createIndex({ status: 1, at: -1 })
    await this.links.createIndex({ entry: 1 }, { unique: true })
    this.indexed = true
  }

  async record(entry: HoldInput): Promise<Hold> {
    await this.ready()
    const hold = complete(entry)
    await this.holds.insertOne(hold)
    return hold
  }

  async list(options: { status?: HoldStatus; limit?: number } = {}): Promise<Hold[]> {
    await this.ready()
    return this.holds
      .find(options.status ? { status: options.status } : {}, { projection: { _id: 0 } })
      .sort({ at: -1 })
      .limit(Math.min(Math.max(options.limit ?? 100, 1), 500))
      .toArray()
  }

  async get(id: string): Promise<Hold | undefined> {
    await this.ready()
    return (await this.holds.findOne({ id }, { projection: { _id: 0 } })) ?? undefined
  }

  async decide(
    id: string,
    decision: 'approved' | 'rejected',
    detail: { note?: string; effect?: string } = {},
  ): Promise<Hold | undefined> {
    await this.ready()
    const result = await this.holds.findOneAndUpdate(
      { id },
      {
        $set: {
          status: decision,
          decidedAt: new Date().toISOString(),
          ...(detail.note ? { note: detail.note } : {}),
          ...(detail.effect ? { effect: detail.effect } : {}),
        },
      },
      { returnDocument: 'after', projection: { _id: 0 } },
    )
    return result ?? undefined
  }

  async counts(): Promise<Record<HoldStatus, number>> {
    await this.ready()
    const rows = await this.holds
      .aggregate<{ _id: HoldStatus; total: number }>([{ $group: { _id: '$status', total: { $sum: 1 } } }])
      .toArray()
    const counts: Record<HoldStatus, number> = { pending: 0, approved: 0, rejected: 0 }
    for (const row of rows) counts[row._id] = row.total
    return counts
  }

  async allowedLinks(): Promise<string[]> {
    await this.ready()
    const rows = await this.links.find({}, { projection: { entry: 1 } }).toArray()
    return rows.map((row) => row.entry)
  }

  async allowLink(entry: string, detail: { holdId: string }): Promise<void> {
    await this.ready()
    await this.links.updateOne(
      { entry },
      { $setOnInsert: { entry, holdId: detail.holdId, at: new Date().toISOString() } },
      { upsert: true },
    )
  }

  async close(): Promise<void> {
    await this.client.close()
  }
}

/** For tests and for running without Mongo. */
export class MemoryHoldLog implements HoldLog {
  private readonly entries: Hold[] = []
  private readonly links = new Map<string, AllowedLink>()

  async record(entry: HoldInput): Promise<Hold> {
    const hold = complete(entry)
    this.entries.unshift(hold)
    return hold
  }

  async list(options: { status?: HoldStatus; limit?: number } = {}): Promise<Hold[]> {
    return this.entries
      .filter((hold) => !options.status || hold.status === options.status)
      .slice(0, options.limit ?? 100)
  }

  async get(id: string): Promise<Hold | undefined> {
    return this.entries.find((hold) => hold.id === id)
  }

  async decide(
    id: string,
    decision: 'approved' | 'rejected',
    detail: { note?: string; effect?: string } = {},
  ): Promise<Hold | undefined> {
    const hold = this.entries.find((entry) => entry.id === id)
    if (!hold) return undefined
    hold.status = decision
    hold.decidedAt = new Date().toISOString()
    if (detail.note) hold.note = detail.note
    if (detail.effect) hold.effect = detail.effect
    return hold
  }

  async counts(): Promise<Record<HoldStatus, number>> {
    const counts: Record<HoldStatus, number> = { pending: 0, approved: 0, rejected: 0 }
    for (const hold of this.entries) counts[hold.status] += 1
    return counts
  }

  async allowedLinks(): Promise<string[]> {
    return [...this.links.keys()]
  }

  async allowLink(entry: string, detail: { holdId: string }): Promise<void> {
    if (!this.links.has(entry)) {
      this.links.set(entry, { entry, holdId: detail.holdId, at: new Date().toISOString() })
    }
  }
}
