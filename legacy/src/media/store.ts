import { mkdirSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import type { StoredMedia } from './types.js'

const EXTENSIONS: Record<StoredMedia['mimeType'], string> = {
  'image/png': 'png',
  'image/jpeg': 'jpg',
  'image/webp': 'webp',
  'video/mp4': 'mp4',
}

/**
 * Holds generated media between the tool that made it and the code that
 * attaches it to a post or a chat message.
 *
 * In-process on purpose. A generated image lives for one turn: the drafter
 * asks for it, names it in its answer, and the job attaches it minutes later
 * at most. Anything older than the TTL is somebody's abandoned draft, and the
 * cap keeps a chatty day from holding a few hundred megabytes of PNGs. A copy
 * is written to disk when a directory is given, the way charts are, so a
 * person can see what the account actually made.
 */
export class MediaStore {
  private readonly items = new Map<string, StoredMedia>()
  private counter = 0

  constructor(
    private readonly options: {
      outputDir?: string
      ttlMs?: number
      maxItems?: number
      now?: () => number
    } = {},
  ) {}

  private now(): number {
    return (this.options.now ?? Date.now)()
  }

  put(media: Omit<StoredMedia, 'id' | 'createdAt' | 'path'>): StoredMedia {
    this.sweep()
    this.counter += 1
    const prefix = media.kind === 'video' ? 'vid' : 'img'
    const id = `${prefix}_${this.now().toString(36)}_${this.counter}`
    const stored: StoredMedia = { ...media, id, createdAt: this.now() }
    const path = this.persist(stored)
    if (path) stored.path = path
    this.items.set(id, stored)
    return stored
  }

  get(id: string): StoredMedia | undefined {
    this.sweep()
    return this.items.get(id)
  }

  /** Every item still held, oldest first. */
  list(): StoredMedia[] {
    this.sweep()
    return [...this.items.values()]
  }

  private sweep(): void {
    const ttl = this.options.ttlMs ?? 60 * 60_000
    const max = this.options.maxItems ?? 50
    const cutoff = this.now() - ttl
    for (const [id, item] of this.items) {
      if (item.createdAt < cutoff) this.items.delete(id)
    }
    // Insertion order is age order, so the front of the map is the oldest.
    while (this.items.size >= max) {
      const oldest = this.items.keys().next().value
      if (oldest === undefined) break
      this.items.delete(oldest)
    }
  }

  private persist(media: StoredMedia): string | undefined {
    const dir = this.options.outputDir
    if (!dir) return undefined
    try {
      mkdirSync(dir, { recursive: true })
      const path = join(dir, `${media.id}.${EXTENSIONS[media.mimeType]}`)
      writeFileSync(path, media.data)
      return path
    } catch {
      // The copy is a convenience. Failing the generation over a full disk
      // would turn a missing file into a missing reply.
      return undefined
    }
  }
}

/**
 * How much the account may generate per day, counted per kind because a
 * video costs a hundred times what an image does.
 *
 * A public bot can be asked "make me a video" by anybody, any number of
 * times, and the prompt asking it to be sparing is a request, not a limit.
 * This is the limit. It resets at UTC midnight and lives in memory: a restart
 * forgets the count, which errs towards generosity by at most one day's cap.
 */
export class MediaBudget {
  private day = ''
  private readonly used = { image: 0, video: 0 }

  constructor(
    private readonly limits: { imagesPerDay: number; videosPerDay: number },
    private readonly now: () => number = Date.now,
  ) {}

  private roll(): void {
    const today = new Date(this.now()).toISOString().slice(0, 10)
    if (today !== this.day) {
      this.day = today
      this.used.image = 0
      this.used.video = 0
    }
  }

  remaining(kind: 'image' | 'video'): number {
    this.roll()
    const limit = kind === 'image' ? this.limits.imagesPerDay : this.limits.videosPerDay
    return Math.max(0, limit - this.used[kind])
  }

  /** Takes one from today's allowance, or returns false when there is none left. */
  take(kind: 'image' | 'video'): boolean {
    if (this.remaining(kind) <= 0) return false
    this.used[kind] += 1
    return true
  }
}
