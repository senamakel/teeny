import { createHash } from 'node:crypto'
import { MongoClient, type Collection } from 'mongodb'
import type { XOwnPostMetrics, XPost, XSearchOptions, XTools, XTrend } from '../types.js'

/**
 * A read-through cache in front of the X API.
 *
 * X bills every post a read returns, on top of the request itself, and the
 * same reads recur: a thread's root and its replies are fetched once per
 * mention under it, the quote job runs the same two searches every hour, the
 * agent asks for a post it was just shown in a search. None of that is worth
 * paying for twice. Content is the part that never changes -- a post's text,
 * author and media are fixed the moment it is published -- so a post, once
 * seen through any read, is kept for a long while and served from here. Lists
 * (searches, a conversation, trends) and counters do change, and those are
 * kept only briefly: long enough to absorb a job that asks the same thing
 * several times in one run, short enough that nothing reads as stale.
 *
 * Every read is also a single flight: two callers asking the same thing at
 * once share one request. Writes never come near this layer.
 */

export type XCacheKind = 'post' | 'missing' | 'search' | 'conversation' | 'trends' | 'metrics' | 'follows'

export interface XCacheTtl extends Record<XCacheKind, number> {}

/** How long each kind of answer is trusted, in milliseconds. */
export const DEFAULT_X_CACHE_TTL: XCacheTtl = {
  /** A post's content cannot change. Metrics carried on it are a snapshot. */
  post: 30 * 24 * 3_600_000,
  /** A post X said was not there: deleted, protected, or not yet visible. */
  missing: 3_600_000,
  search: 10 * 60_000,
  conversation: 10 * 60_000,
  trends: 30 * 60_000,
  /** The account's own counters: the heat job reads them every 20 minutes. */
  metrics: 10 * 60_000,
  /** Whether somebody follows the account. */
  follows: 24 * 3_600_000,
}

export interface XCacheEntry<T = unknown> {
  key: string
  kind: XCacheKind
  value: T
  storedAt: string
  expiresAt: Date
}

export interface XCacheStore {
  get<T>(key: string): Promise<XCacheEntry<T> | undefined>
  set<T>(entry: XCacheEntry<T>): Promise<void>
  close(): Promise<void>
}

/**
 * Durable, so a restart does not re-buy what the last process already paid
 * for. Mongo expires rows itself through the index on `expiresAt`; a row it
 * has not swept yet is still checked against the clock on the way out.
 */
export class MongoXCache implements XCacheStore {
  private readonly client: MongoClient
  private readonly rows: Collection<XCacheEntry>
  private indexed = false

  constructor(options: { uri: string; database: string; collection?: string }) {
    this.client = new MongoClient(options.uri)
    this.rows = this.client.db(options.database).collection<XCacheEntry>(options.collection ?? 'x-cache')
  }

  private async ready(): Promise<void> {
    if (this.indexed) return
    await this.client.connect()
    await this.rows.createIndex({ key: 1 }, { unique: true })
    await this.rows.createIndex({ expiresAt: 1 }, { expireAfterSeconds: 0 })
    this.indexed = true
  }

  async get<T>(key: string): Promise<XCacheEntry<T> | undefined> {
    await this.ready()
    const row = await this.rows.findOne({ key }, { projection: { _id: 0 } })
    return (row as XCacheEntry<T> | null) ?? undefined
  }

  async set<T>(entry: XCacheEntry<T>): Promise<void> {
    await this.ready()
    await this.rows.replaceOne({ key: entry.key }, entry, { upsert: true })
  }

  async close(): Promise<void> {
    await this.client.close()
  }
}

export class InMemoryXCache implements XCacheStore {
  readonly rows = new Map<string, XCacheEntry>()

  async get<T>(key: string): Promise<XCacheEntry<T> | undefined> {
    return this.rows.get(key) as XCacheEntry<T> | undefined
  }

  async set<T>(entry: XCacheEntry<T>): Promise<void> {
    this.rows.set(entry.key, entry)
  }

  async close(): Promise<void> {}
}

/** The reads this layer knows how to keep. Everything else passes straight through. */
export type XReads = Partial<Pick<XTools, 'searchPosts' | 'getPost' | 'getConversation' | 'postMetrics'>> & {
  trends?(options?: { woeid?: number; max?: number }): Promise<XTrend[]>
  followsAccount?(userId: string): Promise<boolean | undefined>
}

export interface XCacheOptions {
  ttl?: Partial<XCacheTtl>
  now?: () => number
  logger?: { debug?(detail: object, event: string): void; warn(detail: object, event: string): void }
}

function digest(value: unknown): string {
  return createHash('sha256').update(JSON.stringify(value)).digest('hex').slice(0, 24)
}

/**
 * Wraps a client so that every cacheable read goes through the store first.
 * The wrapped client's own internals are untouched: `findMentions` reaches
 * `searchPosts` inside the class, not through this proxy, so the intake poll
 * is never served a stale window. What is cached is what the jobs and the
 * agent's tools call from outside.
 */
export function cached<T extends XReads>(x: T, store: XCacheStore, options: XCacheOptions = {}): T {
  const ttl: XCacheTtl = { ...DEFAULT_X_CACHE_TTL, ...options.ttl }
  const now = options.now ?? Date.now
  const inflight = new Map<string, Promise<unknown>>()

  // A store that cannot be read or written is a cache miss, not a failed
  // read: the call goes to X as it would have without the cache.
  const read = async <V>(key: string): Promise<XCacheEntry<V> | undefined> => {
    try {
      const entry = await store.get<V>(key)
      if (!entry) return undefined
      if (new Date(entry.expiresAt).getTime() <= now()) return undefined
      return entry
    } catch (error) {
      options.logger?.warn({ key, err: error }, 'x cache: could not read')
      return undefined
    }
  }
  const write = async <V>(key: string, kind: XCacheKind, value: V): Promise<void> => {
    try {
      await store.set<V>({
        key,
        kind,
        value,
        storedAt: new Date(now()).toISOString(),
        expiresAt: new Date(now() + ttl[kind]),
      })
    } catch (error) {
      options.logger?.warn({ key, err: error }, 'x cache: could not write')
    }
  }

  const through = async <V>(key: string, kind: XCacheKind, fetch: () => Promise<V>): Promise<V> => {
    const hit = await read<V>(key)
    if (hit) {
      options.logger?.debug?.({ key, kind }, 'x cache: hit')
      return hit.value
    }
    const pending = inflight.get(key) as Promise<V> | undefined
    if (pending) return pending
    const flight = (async () => {
      try {
        const value = await fetch()
        await write(key, kind, value)
        return value
      } finally {
        inflight.delete(key)
      }
    })()
    inflight.set(key, flight)
    return flight
  }

  // Every post a list brings back is a post already paid for; remembering
  // each one under its own key is what makes the next `getPost` free.
  const remember = async (posts: XPost[]): Promise<void> => {
    await Promise.all(posts.map((post) => write(`post:${post.id}`, 'post', post)))
  }

  // `startTime`/`endTime` on a search are usually "now minus a window", which
  // would give every call a new key. Rounding them to the search TTL means
  // the same query asked twice inside one window is one request.
  const bucket = (iso: string | undefined): string | undefined => {
    if (!iso) return undefined
    const ms = new Date(iso).getTime()
    if (Number.isNaN(ms)) return iso
    return String(Math.floor(ms / ttl.search))
  }

  const searchPosts = async (query: string, searchOptions: XSearchOptions = {}): Promise<XPost[]> => {
    const key = `search:${digest({
      query,
      maxResults: searchOptions.maxResults,
      sinceId: searchOptions.sinceId,
      sortOrder: searchOptions.sortOrder,
      startTime: bucket(searchOptions.startTime),
      endTime: bucket(searchOptions.endTime),
    })}`
    return through(key, 'search', async () => {
      const posts = await x.searchPosts!(query, searchOptions)
      await remember(posts)
      return posts
    })
  }

  const getPost = async (id: string): Promise<XPost | undefined> => {
    const key = `post:${id}`
    const hit = await read<XPost>(key)
    if (hit) return hit.value
    const missing = await read<null>(`missing:${id}`)
    if (missing) return undefined
    const pending = inflight.get(key) as Promise<XPost | undefined> | undefined
    if (pending) return pending
    const flight = (async () => {
      try {
        const post = await x.getPost!(id)
        if (post) await write(key, 'post', post)
        else await write(`missing:${id}`, 'missing', null)
        return post
      } finally {
        inflight.delete(key)
      }
    })()
    inflight.set(key, flight)
    return flight
  }

  const getConversation = async (conversationId: string, maxResults?: number): Promise<XPost[]> => {
    const key = `conversation:${conversationId}:${maxResults ?? ''}`
    return through(key, 'conversation', async () => {
      const posts = await x.getConversation!(conversationId, maxResults)
      await remember(posts)
      return posts
    })
  }

  const trends = async (trendOptions: { woeid?: number; max?: number } = {}): Promise<XTrend[]> => {
    const key = `trends:${trendOptions.woeid ?? 1}:${trendOptions.max ?? ''}`
    return through(key, 'trends', () => x.trends!(trendOptions))
  }

  const postMetrics = async (ids: string[]): Promise<XOwnPostMetrics[]> => {
    if (ids.length === 0) return []
    const key = `metrics:${digest([...ids].sort())}`
    return through(key, 'metrics', () => x.postMetrics!(ids))
  }

  const followsAccount = async (userId: string): Promise<boolean | undefined> => {
    const key = `follows:${userId}`
    const hit = await read<boolean>(key)
    if (hit) return hit.value
    const answer = await x.followsAccount!(userId)
    // An unknown answer is a failed lookup, not a fact; it is asked again next time.
    if (answer !== undefined) await write(key, 'follows', answer)
    return answer
  }

  const overrides: Record<string, unknown> = {
    ...(typeof x.searchPosts === 'function' ? { searchPosts } : {}),
    ...(typeof x.getPost === 'function' ? { getPost } : {}),
    ...(typeof x.getConversation === 'function' ? { getConversation } : {}),
    ...(typeof x.trends === 'function' ? { trends } : {}),
    ...(typeof x.postMetrics === 'function' ? { postMetrics } : {}),
    ...(typeof x.followsAccount === 'function' ? { followsAccount } : {}),
  }

  return new Proxy(x, {
    get(target, property, receiver) {
      if (typeof property === 'string' && property in overrides) return overrides[property]
      const value = Reflect.get(target, property, receiver) as unknown
      return typeof value === 'function' ? (value as (...args: unknown[]) => unknown).bind(target) : value
    },
  })
}
