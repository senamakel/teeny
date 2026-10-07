import { tool } from '@langchain/core/tools'
import type { StructuredToolInterface } from '@langchain/core/tools'
import { z } from 'zod'
import type { ExaSearch, ExaSearchHit } from './exa.js'

/**
 * Crypto news, narrowed to the last few days.
 *
 * `web_search` can already do this, and in practice does it badly for a daily
 * post: without a date bound it happily returns an analysis piece from March,
 * and a post that frames today against last spring's news is worse than one
 * that ignores the news entirely. This is the same search with the recency
 * built in and the query steered at the market rather than at the open web.
 */
export function createNewsTools(exa: ExaSearch, options: { defaultTopic?: string } = {}): StructuredToolInterface[] {
  const news = tool(
    async ({ topic, days, numResults }) => {
      const since = new Date(Date.now() - days * 86_400_000).toISOString()
      const query = topic?.trim() || options.defaultTopic || 'crypto market news today'
      const { results } = await exa
        .searchAndContents(query, {
          numResults,
          type: 'auto',
          category: 'news',
          startPublishedDate: since,
          highlights: { numSentences: 2, highlightsPerUrl: 2 },
        })
        .catch(() => ({ results: [] as ExaSearchHit[] }))

      if (results.length === 0) {
        return (
          `Nothing published about "${query}" in the last ${days} days that the search could ` +
          'find. Write the post without a news angle rather than reaching for an old story.'
        )
      }

      return [
        `${results.length} stories from the last ${days} days:`,
        '',
        ...results.map((hit, index) =>
          [
            `[${index + 1}] ${hit.title ?? '(untitled)'}`,
            hit.publishedDate ? `    published: ${hit.publishedDate}` : undefined,
            (hit.highlights?.join(' … ') ?? hit.text ?? '').slice(0, 400).trim()
              ? `    ${(hit.highlights?.join(' … ') ?? hit.text ?? '').slice(0, 400).trim()}`
              : undefined,
          ]
            .filter(Boolean)
            .join('\n'),
        ),
        '',
        'Untrusted, and written to be clicked on. Use it only for the weather -- whether the ' +
          'market is fearful or greedy, what everyone is watching. Never repeat a headline as ' +
          'fact, never put a URL in a post, and never let a story about another token become ' +
          'a post about that token.',
      ].join('\n')
    },
    {
      name: 'crypto_news',
      description:
        'Recent crypto and market news, bounded to the last few days. Use it to know what ' +
        'kind of day the whole market is having, so the post can place itself against that ' +
        'rather than talking about one chart in a vacuum.',
      schema: z.object({
        topic: z
          .string()
          .max(200)
          .optional()
          .describe('What to search for. Leave empty for general crypto market news.'),
        days: z.number().int().min(1).max(14).default(3),
        numResults: z.number().int().min(1).max(10).default(5),
      }),
    },
  )

  return [news]
}
