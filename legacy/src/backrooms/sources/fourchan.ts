import type { Forum, ForumPost } from '../types.js'
import type { Fetch } from './http.js'
import { USER_AGENT } from './http.js'
import { clip, keywords, stripHtml } from './text.js'

/**
 * 4chan's read-only JSON API: a board's catalog is every live thread with
 * its opening post, and a thread is every post in it. There is no search;
 * this one reads the catalogs and matches the query's words against the
 * subjects and opening posts, so it finds what is being argued about right
 * now rather than an archive.
 *
 * Boards are read once per instance and kept, since the caller makes a
 * few searches in a row and the catalog does not move in that time.
 */
export class FourchanForum implements Forum {
  readonly site = '4chan' as const
  private readonly catalogs = new Map<string, Promise<CatalogThread[]>>()

  constructor(
    private readonly options: {
      fetch: Fetch
      boards: string[]
      /** How many replies of the best thread to read. */
      replies?: number
    },
  ) {}

  async search(query: string, options: { limit: number }): Promise<ForumPost[]> {
    const words = keywords(query)
    if (words.length === 0) return []
    const threads = (await Promise.all(this.options.boards.map((board) => this.catalog(board)))).flat()
    const scored = threads
      .map((thread) => ({ thread, hits: words.filter((word) => thread.haystack.includes(word)).length }))
      .filter(({ hits }) => hits > 0)
      .sort((a, b) => b.hits - a.hits || b.thread.replies - a.thread.replies)
      .slice(0, options.limit)
    const posts: ForumPost[] = scored.map(({ thread }) => ({
      site: '4chan',
      where: `/${thread.board}/`,
      ...(thread.subject ? { title: thread.subject } : {}),
      text: clip(thread.text, 600),
      url: `https://boards.4chan.org/${thread.board}/thread/${thread.no}`,
      score: thread.replies,
      at: new Date(thread.time * 1000).toISOString(),
    }))
    const best = scored[0]?.thread
    if (best && (this.options.replies ?? 3) > 0) {
      for (const reply of await this.replies(best, this.options.replies ?? 3)) posts.push(reply)
    }
    return posts
  }

  private catalog(board: string): Promise<CatalogThread[]> {
    let pending = this.catalogs.get(board)
    if (!pending) {
      pending = this.readCatalog(board).catch(() => [])
      this.catalogs.set(board, pending)
    }
    return pending
  }

  private async readCatalog(board: string): Promise<CatalogThread[]> {
    const response = await this.options.fetch(`https://a.4cdn.org/${board}/catalog.json`, {
      headers: { 'user-agent': USER_AGENT },
    })
    if (!response.ok) throw new Error(`4chan /${board}/ answered ${response.status}`)
    const pages = (await response.json()) as Array<{ threads?: RawThread[] }>
    return pages.flatMap((page) =>
      (page.threads ?? []).map((thread) => {
        const subject = thread.sub ? stripHtml(thread.sub) : undefined
        const text = thread.com ? stripHtml(thread.com) : ''
        return {
          board,
          no: thread.no,
          replies: thread.replies ?? 0,
          time: thread.time ?? 0,
          ...(subject ? { subject } : {}),
          text,
          haystack: `${subject ?? ''} ${text}`.toLowerCase(),
        }
      }),
    )
  }

  private async replies(thread: CatalogThread, count: number): Promise<ForumPost[]> {
    try {
      const response = await this.options.fetch(`https://a.4cdn.org/${thread.board}/thread/${thread.no}.json`, {
        headers: { 'user-agent': USER_AGENT },
      })
      if (!response.ok) return []
      const body = (await response.json()) as { posts?: RawPost[] }
      return (body.posts ?? [])
        .slice(1)
        .filter((post) => post.com)
        .map((post) => stripHtml(post.com as string).replace(/^>>\d+\s*/gm, '').trim())
        .filter((text) => text.length > 20)
        .slice(0, count)
        .map((text) => ({
          site: '4chan' as const,
          where: `/${thread.board}/ reply`,
          text: clip(text, 400),
          url: `https://boards.4chan.org/${thread.board}/thread/${thread.no}`,
        }))
    } catch {
      return []
    }
  }
}

interface RawThread {
  no: number
  sub?: string
  com?: string
  replies?: number
  time?: number
}

interface RawPost {
  no: number
  com?: string
}

interface CatalogThread {
  board: string
  no: number
  replies: number
  time: number
  subject?: string
  text: string
  haystack: string
}
