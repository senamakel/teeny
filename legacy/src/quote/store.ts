import { mkdir, readFile, rename, writeFile } from 'node:fs/promises'
import { dirname } from 'node:path'

/** One post the account has quoted, enough to never quote it or its author again too soon. */
export interface QuotedPost {
  postId: string
  authorUsername?: string
  /** ISO 8601, when the quote went out (or would have, on a dry run). */
  quotedAt: string
  /** The account's own post that carried the quote; absent on a dry run. */
  ownPostId?: string
}

export interface QuoteLog {
  list(): Promise<QuotedPost[]>
  record(entry: QuotedPost): Promise<void>
}

const MAX_KEPT = 2_000

/**
 * The quotes the account has already made, as a JSON file on the data
 * volume.
 *
 * A file rather than memory because the rule it serves is exact: a post
 * quoted twice is a bot showing its seams, and "never the same post again"
 * is not something a recall over prose can promise. Memory still gets the
 * quote, for the voice; this is for the arithmetic. Written atomically, as
 * the mention state is, so a restart mid-write leaves the old list rather
 * than half of the new one.
 */
export class FileQuoteLog implements QuoteLog {
  constructor(private readonly path: string) {}

  async list(): Promise<QuotedPost[]> {
    try {
      const parsed = JSON.parse(await readFile(this.path, 'utf8')) as { quoted?: QuotedPost[] }
      return Array.isArray(parsed.quoted) ? parsed.quoted.filter((entry) => typeof entry?.postId === 'string') : []
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === 'ENOENT') return []
      throw error
    }
  }

  async record(entry: QuotedPost): Promise<void> {
    const quoted = [...(await this.list()).filter((kept) => kept.postId !== entry.postId), entry].slice(-MAX_KEPT)
    await mkdir(dirname(this.path), { recursive: true })
    const temporaryPath = `${this.path}.tmp`
    await writeFile(temporaryPath, `${JSON.stringify({ quoted }, null, 2)}\n`)
    await rename(temporaryPath, this.path)
  }
}

/** For tests and rehearsals: the same contract, nothing on disk. */
export class MemoryQuoteLog implements QuoteLog {
  constructor(private readonly quoted: QuotedPost[] = []) {}

  async list(): Promise<QuotedPost[]> {
    return [...this.quoted]
  }

  async record(entry: QuotedPost): Promise<void> {
    const index = this.quoted.findIndex((kept) => kept.postId === entry.postId)
    if (index >= 0) this.quoted.splice(index, 1)
    this.quoted.push(entry)
  }
}
