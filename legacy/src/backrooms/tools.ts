import { tool } from '@langchain/core/tools'
import type { StructuredToolInterface } from '@langchain/core/tools'
import { z } from 'zod'
import type { ExaSearch, ExaSearchHit } from '../agent/tools/exa.js'
import type { Memory, XPost } from '../types.js'
import { admissible, admissiblePost, renderPost } from './screen.js'
import type { Forum, ToolName } from './types.js'

export interface ToolDeps {
  exa?: ExaSearch
  memory?: Memory
  forums: Forum[]
  x?: { searchPosts(query: string, options: { maxResults: number }): Promise<XPost[]> }
}

/**
 * What an agent in the room can reach for. Read-only, all of it: the room
 * is for finding things out and saying things, not for remembering or
 * posting, so the `remember` and `post` tools the reply agent carries are
 * deliberately not here. Every hit is screened before the agent sees it.
 */
export function backroomsTools(wanted: ToolName[], deps: ToolDeps): StructuredToolInterface[] {
  const tools: StructuredToolInterface[] = []
  const has = (name: ToolName): boolean => wanted.includes(name)

  if (has('web_search') && deps.exa) {
    const exa = deps.exa
    tools.push(
      tool(
        async ({ query, days }) => {
          const { results } = await exa.searchAndContents(query, {
            numResults: 6,
            type: 'auto',
            ...(days ? { startPublishedDate: new Date(Date.now() - days * 86_400_000).toISOString() } : {}),
            highlights: { numSentences: 3, highlightsPerUrl: 1 },
          })
          return renderHits(results.filter((hit) => admissible(`${hit.title ?? ''} ${hit.highlights?.join(' ') ?? hit.text ?? ''}`)))
        },
        {
          name: 'web_search',
          description: 'search the live web. news, papers, weird pages, anything. returns titles, urls and excerpts.',
          schema: z.object({
            query: z.string().min(1),
            days: z.number().int().min(1).max(30).optional().describe('only things published in the last n days'),
          }),
        },
      ),
    )
  }

  if (has('recall_memory') && deps.memory) {
    const memory = deps.memory
    tools.push(
      tool(
        async ({ query }) => {
          const context = await memory.recall(query)
          return context.trim() || 'nothing comes back.'
        },
        {
          name: 'recall_memory',
          description: "search teeny's long-term memory: past posts, past conversations, things people said, things teeny decided.",
          schema: z.object({ query: z.string().min(1) }),
        },
      ),
    )
  }

  for (const forum of deps.forums) {
    const name: ToolName = forum.site === 'reddit' ? 'reddit_search' : 'fourchan_search'
    if (!has(name)) continue
    tools.push(
      tool(
        async ({ query }) => {
          const posts = (await forum.search(query, { limit: 8 })).filter(admissiblePost)
          return posts.length > 0 ? posts.map(renderPost).join('\n\n') : 'nothing on the boards about that.'
        },
        {
          name,
          description:
            forum.site === 'reddit'
              ? 'search reddit (the ai, singularity and conspiracy subs) for what people are saying this week.'
              : 'search the 4chan boards (/g/, /x/, /sci/, /biz/) for what anons are arguing about right now.',
          schema: z.object({ query: z.string().min(1) }),
        },
      ),
    )
  }

  if (has('x_search') && deps.x) {
    const x = deps.x
    tools.push(
      tool(
        async ({ query }) => {
          const posts = await x.searchPosts(`${query} -is:retweet`, { maxResults: 10 })
          const lines = posts
            .filter((post) => admissible(post.text))
            .map((post) => `@${post.authorUsername ?? 'someone'}: ${post.text.replace(/\s+/g, ' ')}`)
          return lines.length > 0 ? lines.join('\n') : 'nobody on x is saying that.'
        },
        {
          name: 'x_search',
          description: 'search recent posts on x.',
          schema: z.object({ query: z.string().min(1) }),
        },
      ),
    )
  }

  return tools
}

function renderHits(hits: ExaSearchHit[]): string {
  if (hits.length === 0) return 'nothing out there.'
  return hits
    .map((hit) => {
      const body = (hit.highlights?.join(' … ') ?? hit.text ?? '').slice(0, 500).trim()
      const source = safeHost(hit.url)
      return `[${source}${hit.publishedDate ? ` ${hit.publishedDate.slice(0, 10)}` : ''}] ${hit.title ?? '(untitled)'}${body ? ` — ${body}` : ''}`
    })
    .join('\n\n')
}

export function safeHost(url: string): string {
  try {
    return new URL(url).hostname.replace(/^www\./, '')
  } catch {
    return url
  }
}
