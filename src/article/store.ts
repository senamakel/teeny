import { mkdir, readdir, readFile, rename, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import { z } from 'zod'
import type { Craziness } from '../backrooms/evaluate.js'
import type { Thought } from '../backrooms/distill.js'
import type { Seed } from '../backrooms/types.js'
import type { XArticleContentState } from '../types.js'
import { proofSchema } from './proofread.js'
import { structureCheckSchema, structureSchema } from './stages.js'

/**
 * Where articles live: one directory per article under ARTICLES_DIR, with
 * `article.json` and the pictures as files beside it. The record carries
 * every stage's output (the room, the structure and its check, the proof
 * findings, the problems) so a rehearsal can be read file by file, and the
 * bot serves the same directory to the site.
 */

export const SLUG = /^[a-z0-9-]+$/
export const IMAGE_FILE = /^[a-z0-9-]+\.(png|jpg|webp)$/

export const articleImageSchema = z.object({
  /** `cover` or the section key. */
  key: z.string().min(1),
  file: z.string().regex(IMAGE_FILE),
  mimeType: z.enum(['image/png', 'image/jpeg', 'image/webp']),
  prompt: z.string(),
  caption: z.string().default(''),
  /** X's media id once uploaded. */
  mediaId: z.string().optional(),
})

export type ArticleImage = z.infer<typeof articleImageSchema>

export const articleStatusSchema = z.enum(['structure', 'draft', 'dry-run', 'held', 'skipped', 'published', 'failed'])
export type ArticleStatus = z.infer<typeof articleStatusSchema>

export const articleRecordSchema = z.object({
  slug: z.string().regex(SLUG),
  title: z.string(),
  dek: z.string(),
  status: articleStatusSchema,
  createdAt: z.string(),
  publishedAt: z.string().optional(),
  /** The room: what it was about, what came through the wire, what it thought. */
  engine: z.string().optional(),
  seed: z.custom<Seed>().optional(),
  runId: z.string().optional(),
  /** Where the room's transcript was written, for `--from`. */
  runPath: z.string().optional(),
  thought: z.custom<Thought>().optional(),
  judge: z.custom<Craziness>().optional(),
  rooms: z.array(z.object({ runId: z.string().optional(), total: z.number().optional(), rejected: z.string().optional() })).default([]),
  /** The plan, and what the editor said about it. */
  structure: structureSchema.optional(),
  structureCheck: structureCheckSchema.optional(),
  /** The prose: each section as written, and the whole after the proof-reader and the style pass. */
  sections: z.array(z.object({ key: z.string(), heading: z.string(), markdown: z.string() })).default([]),
  markdown: z.string().default(''),
  wordCount: z.number().default(0),
  /** What the flow editor saw, and whether the rewrite was taken. */
  flow: z
    .object({
      review: z.object({ ok: z.boolean(), problems: z.array(z.string()).default([]), plan: z.array(z.string()).default([]) }),
      applied: z.boolean(),
      refused: z.string().optional(),
    })
    .optional(),
  proof: z
    .object({
      review: proofSchema,
      applied: z.boolean(),
      refused: z.string().optional(),
      after: proofSchema.optional(),
    })
    .optional(),
  critique: z.string().optional(),
  cover: articleImageSchema.optional(),
  images: z.array(articleImageSchema).default([]),
  /** The body as X takes it, from the last run that got that far; media ids are placeholders on a rehearsal. */
  contentState: z.custom<XArticleContentState>().optional(),
  /** What X gave back. */
  xArticleId: z.string().optional(),
  postId: z.string().optional(),
  error: z.string().optional(),
  problems: z.array(z.string()).default([]),
})

export type ArticleRecord = z.infer<typeof articleRecordSchema>

export interface ArticleSummary {
  slug: string
  title: string
  dek: string
  status: ArticleStatus
  createdAt: string
  publishedAt?: string
  postId?: string
  cover?: string
  wordCount: number
}

export interface ArticleStore {
  /** Published articles, newest first; everything with `all`. */
  list(options?: { all?: boolean; limit?: number }): Promise<ArticleSummary[]>
  get(slug: string): Promise<ArticleRecord | undefined>
  /** The published article that a post on X carried, by that post's id. */
  findByPostId(postId: string): Promise<ArticleRecord | undefined>
  /** Writes the record, and any files given, under the article's directory. */
  save(record: ArticleRecord, files?: Array<{ file: string; data: Uint8Array }>): Promise<void>
  media(slug: string, file: string): Promise<{ data: Uint8Array; mimeType: string } | undefined>
  latestPublishedAt(): Promise<string | undefined>
  /** The engine of the newest article that got as far as a structure, so the next one picks another. */
  lastEngine(): Promise<string | undefined>
}

export function summarise(record: ArticleRecord): ArticleSummary {
  return {
    slug: record.slug,
    title: record.title,
    dek: record.dek,
    status: record.status,
    createdAt: record.createdAt,
    ...(record.publishedAt ? { publishedAt: record.publishedAt } : {}),
    ...(record.postId ? { postId: record.postId } : {}),
    ...(record.cover ? { cover: record.cover.file } : {}),
    wordCount: record.wordCount,
  }
}

/** Only a published record counts: a post id on anything else is not a post anyone can be replying under. */
function findPublished(records: ArticleRecord[], postId: string): ArticleRecord | undefined {
  if (!postId) return undefined
  return records.find((record) => record.status === 'published' && record.postId === postId)
}

const MIME: Record<string, string> = { png: 'image/png', jpg: 'image/jpeg', webp: 'image/webp' }

export class FileArticleStore implements ArticleStore {
  constructor(private readonly dir: string) {}

  async list(options: { all?: boolean; limit?: number } = {}): Promise<ArticleSummary[]> {
    const records = await this.records()
    const kept = options.all ? records : records.filter((record) => record.status === 'published')
    return kept.slice(0, options.limit ?? kept.length).map(summarise)
  }

  async get(slug: string): Promise<ArticleRecord | undefined> {
    if (!SLUG.test(slug)) return undefined
    try {
      const parsed = articleRecordSchema.safeParse(JSON.parse(await readFile(join(this.dir, slug, 'article.json'), 'utf8')))
      return parsed.success ? parsed.data : undefined
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === 'ENOENT') return undefined
      throw error
    }
  }

  async findByPostId(postId: string): Promise<ArticleRecord | undefined> {
    return findPublished(await this.records(), postId)
  }

  async save(record: ArticleRecord, files: Array<{ file: string; data: Uint8Array }> = []): Promise<void> {
    if (!SLUG.test(record.slug)) throw new Error(`not a slug: ${record.slug}`)
    const dir = join(this.dir, record.slug)
    await mkdir(dir, { recursive: true })
    for (const { file, data } of files) {
      if (!IMAGE_FILE.test(file)) throw new Error(`not an image file name: ${file}`)
      await writeFile(join(dir, file), data)
    }
    const path = join(dir, 'article.json')
    await writeFile(`${path}.tmp`, `${JSON.stringify(record, null, 2)}\n`)
    await rename(`${path}.tmp`, path)
  }

  async media(slug: string, file: string): Promise<{ data: Uint8Array; mimeType: string } | undefined> {
    if (!SLUG.test(slug) || !IMAGE_FILE.test(file)) return undefined
    try {
      const data = await readFile(join(this.dir, slug, file))
      return { data, mimeType: MIME[file.split('.').pop()!] ?? 'application/octet-stream' }
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === 'ENOENT') return undefined
      throw error
    }
  }

  async latestPublishedAt(): Promise<string | undefined> {
    const records = await this.records()
    return records.find((record) => record.status === 'published' && record.publishedAt)?.publishedAt
  }

  async lastEngine(): Promise<string | undefined> {
    const records = await this.records()
    return records.find((record) => record.structure)?.engine
  }

  /** Every record, newest first by creation. */
  private async records(): Promise<ArticleRecord[]> {
    let names: string[]
    try {
      names = (await readdir(this.dir, { withFileTypes: true })).filter((entry) => entry.isDirectory() && SLUG.test(entry.name)).map((entry) => entry.name)
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === 'ENOENT') return []
      throw error
    }
    const records = await Promise.all(names.map((name) => this.get(name)))
    return records.filter((record): record is ArticleRecord => record !== undefined).sort((a, b) => b.createdAt.localeCompare(a.createdAt))
  }
}

export class MemoryArticleStore implements ArticleStore {
  readonly records = new Map<string, ArticleRecord>()
  readonly files = new Map<string, Uint8Array>()

  async list(options: { all?: boolean; limit?: number } = {}): Promise<ArticleSummary[]> {
    const all = [...this.records.values()].sort((a, b) => b.createdAt.localeCompare(a.createdAt))
    const kept = options.all ? all : all.filter((record) => record.status === 'published')
    return kept.slice(0, options.limit ?? kept.length).map(summarise)
  }

  async get(slug: string): Promise<ArticleRecord | undefined> {
    return this.records.get(slug)
  }

  async findByPostId(postId: string): Promise<ArticleRecord | undefined> {
    return findPublished([...this.records.values()], postId)
  }

  async save(record: ArticleRecord, files: Array<{ file: string; data: Uint8Array }> = []): Promise<void> {
    this.records.set(record.slug, structuredClone(record))
    for (const { file, data } of files) this.files.set(`${record.slug}/${file}`, data)
  }

  async media(slug: string, file: string): Promise<{ data: Uint8Array; mimeType: string } | undefined> {
    const data = this.files.get(`${slug}/${file}`)
    return data ? { data, mimeType: MIME[file.split('.').pop()!] ?? 'application/octet-stream' } : undefined
  }

  async latestPublishedAt(): Promise<string | undefined> {
    return (await this.list()).find((record) => record.publishedAt)?.publishedAt
  }

  async lastEngine(): Promise<string | undefined> {
    return [...this.records.values()].sort((a, b) => b.createdAt.localeCompare(a.createdAt)).find((record) => record.structure)?.engine
  }
}

/** `YYYYMMDD-<title>`, the title cleaned the way a backrooms run id is. */
export function slugFor(now: Date, title: string): string {
  const stamp = now.toISOString().slice(0, 10).replace(/-/g, '')
  const clean = title
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 48)
    .replace(/-+$/, '')
  return clean ? `${stamp}-${clean}` : stamp
}

/** The slug, with a `-2`, `-3`… when the store already has it. */
export async function freeSlug(store: Pick<ArticleStore, 'get'>, slug: string): Promise<string> {
  if (!(await store.get(slug))) return slug
  for (let n = 2; n < 100; n += 1) {
    const candidate = `${slug}-${n}`
    if (!(await store.get(candidate))) return candidate
  }
  throw new Error(`no free slug for ${slug}`)
}
