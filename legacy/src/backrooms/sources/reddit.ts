import type { ExaSearch } from '../../agent/tools/exa.js'
import type { Forum, ForumPost } from '../types.js'
import type { Fetch } from './http.js'
import { USER_AGENT } from './http.js'
import { clip, unescapeEntities } from './text.js'

/**
 * Reddit's public JSON: any listing url with `.json` on the end. It works
 * without a token and with a real user agent, until it does not -- reddit
 * throttles anonymous readers with 403s and 429s at will. When it does, and
 * Exa is around, the same query goes to Exa restricted to reddit.com, which
 * is slower to update but never closed.
 */
export class RedditForum implements Forum {
  readonly site = 'reddit' as const

  constructor(
    private readonly options: {
      fetch: Fetch
      subs: string[]
      exa?: ExaSearch
    },
  ) {}

  async search(query: string, options: { limit: number }): Promise<ForumPost[]> {
    const perSub = Math.max(3, Math.ceil(options.limit / Math.max(1, this.options.subs.length)))
    const results = await Promise.all(this.options.subs.map((sub) => this.searchSub(sub, query, perSub)))
    const direct = results.flatMap((result) => result.posts)
    const blocked = results.some((result) => result.blocked)
    if (direct.length === 0 && blocked && this.options.exa) {
      return this.viaExa(query, options.limit)
    }
    return direct.sort((a, b) => (b.score ?? 0) - (a.score ?? 0)).slice(0, options.limit)
  }

  private async searchSub(sub: string, query: string, limit: number): Promise<{ posts: ForumPost[]; blocked: boolean }> {
    const url =
      `https://www.reddit.com/r/${encodeURIComponent(sub)}/search.json?q=${encodeURIComponent(query)}` +
      `&restrict_sr=1&sort=new&t=week&limit=${limit}`
    try {
      const response = await this.options.fetch(url, { headers: { 'user-agent': USER_AGENT, accept: 'application/json' } })
      if (response.status === 403 || response.status === 429) return { posts: [], blocked: true }
      if (!response.ok) return { posts: [], blocked: false }
      const body = (await response.json()) as { data?: { children?: Array<{ data?: RawPost }> } }
      const posts = (body.data?.children ?? [])
        .map((child) => child.data)
        .filter((post): post is RawPost => Boolean(post?.title))
        .map((post) => ({
          site: 'reddit' as const,
          where: `r/${post.subreddit ?? sub}`,
          title: unescapeEntities(post.title),
          text: clip(unescapeEntities(post.selftext ?? '').trim() || unescapeEntities(post.title), 600),
          url: `https://www.reddit.com${post.permalink ?? ''}`,
          ...(typeof post.score === 'number' ? { score: post.score } : {}),
          ...(typeof post.created_utc === 'number' ? { at: new Date(post.created_utc * 1000).toISOString() } : {}),
        }))
      return { posts, blocked: false }
    } catch {
      return { posts: [], blocked: false }
    }
  }

  private async viaExa(query: string, limit: number): Promise<ForumPost[]> {
    try {
      const { results } = await this.options.exa!.searchAndContents(query, {
        numResults: limit,
        type: 'auto',
        includeDomains: ['reddit.com'],
        startPublishedDate: new Date(Date.now() - 7 * 86_400_000).toISOString(),
        highlights: { numSentences: 3, highlightsPerUrl: 1 },
      })
      return results.map((hit) => {
        const sub = /reddit\.com\/(r\/[^/]+)/i.exec(hit.url)?.[1] ?? 'reddit'
        return {
          site: 'reddit' as const,
          where: sub,
          ...(hit.title ? { title: hit.title } : {}),
          text: clip((hit.highlights?.join(' ') || hit.text || hit.title || '').trim(), 600),
          url: hit.url,
          ...(hit.publishedDate ? { at: hit.publishedDate } : {}),
        }
      })
    } catch {
      return []
    }
  }
}

interface RawPost {
  title: string
  selftext?: string
  permalink?: string
  score?: number
  created_utc?: number
  subreddit?: string
}
