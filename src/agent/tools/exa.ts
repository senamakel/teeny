import { tool } from '@langchain/core/tools'
import type { StructuredToolInterface } from '@langchain/core/tools'
import { Exa } from 'exa-js'
import { z } from 'zod'

export interface ExaSearchHit {
  title?: string | null
  url: string
  publishedDate?: string | null
  author?: string | null
  text?: string
  highlights?: string[]
}

/** The slice of exa-js the tools use, kept narrow so tests can fake it. */
export interface ExaSearch {
  searchAndContents(
    query: string,
    options: Record<string, unknown>,
  ): Promise<{ results: ExaSearchHit[] }>
  getContents(
    urls: string[],
    options: Record<string, unknown>,
  ): Promise<{ results: ExaSearchHit[] }>
  findSimilarAndContents(
    url: string,
    options: Record<string, unknown>,
  ): Promise<{ results: ExaSearchHit[] }>
  answer(
    query: string,
    options?: Record<string, unknown>,
  ): Promise<{ answer: string; citations?: Array<{ url: string; title?: string | null }> }>
}

export function createExaClient(apiKey: string): ExaSearch {
  return new Exa(apiKey) as unknown as ExaSearch
}

function renderHits(hits: ExaSearchHit[]): string {
  if (hits.length === 0) return 'No results.'
  return hits
    .map((hit, index) => {
      const body = (hit.highlights?.join(' … ') ?? hit.text ?? '').slice(0, 1_200).trim()
      return [
        `[${index + 1}] ${hit.title ?? '(untitled)'}`,
        `url: ${hit.url}`,
        hit.publishedDate ? `published: ${hit.publishedDate}` : undefined,
        body ? `excerpt: ${body}` : undefined,
      ]
        .filter(Boolean)
        .join('\n')
    })
    .join('\n\n')
}

export function createExaTools(exa: ExaSearch): StructuredToolInterface[] {
  const webSearch = tool(
    async ({ query, numResults, category, startPublishedDate }) => {
      const { results } = await exa.searchAndContents(query, {
        numResults,
        ...(category ? { category } : {}),
        ...(startPublishedDate ? { startPublishedDate } : {}),
        text: { maxCharacters: 1_200 },
        highlights: true,
      })
      return renderHits(results)
    },
    {
      name: 'web_search',
      description:
        'Search the live web with Exa and return titles, urls, and excerpts. Use for news, ' +
        'prices, project background, and anything that may have changed recently.',
      schema: z.object({
        query: z.string().min(1).describe('A natural-language search query.'),
        numResults: z.number().int().min(1).max(10).default(5),
        category: z
          .enum(['company', 'research paper', 'news', 'tweet', 'pdf', 'github', 'financial report'])
          .optional()
          .describe('Optional Exa result category filter.'),
        startPublishedDate: z
          .string()
          .optional()
          .describe('ISO 8601 date; only return results published on or after it.'),
      }),
    },
  )

  const readPages = tool(
    async ({ urls }) => {
      const { results } = await exa.getContents(urls, { text: { maxCharacters: 4_000 } })
      return renderHits(results)
    },
    {
      name: 'read_web_pages',
      description: 'Fetch the readable text of specific urls found by web_search.',
      schema: z.object({
        urls: z.array(z.url()).min(1).max(5),
      }),
    },
  )

  const findSimilar = tool(
    async ({ url, numResults }) => {
      const { results } = await exa.findSimilarAndContents(url, {
        numResults,
        text: { maxCharacters: 1_200 },
      })
      return renderHits(results)
    },
    {
      name: 'find_similar_pages',
      description: 'Find pages similar to a url, for broadening research around one good source.',
      schema: z.object({
        url: z.url(),
        numResults: z.number().int().min(1).max(10).default(5),
      }),
    },
  )

  const answer = tool(
    async ({ question }) => {
      const result = await exa.answer(question)
      const citations = (result.citations ?? [])
        .map((citation) => `- ${citation.title ?? citation.url}: ${citation.url}`)
        .join('\n')
      return citations ? `${result.answer}\n\nSources:\n${citations}` : result.answer
    },
    {
      name: 'web_answer',
      description:
        'Ask Exa a factual question and get a cited answer. Cheaper than searching and ' +
        'reading when one specific fact is needed.',
      schema: z.object({ question: z.string().min(1) }),
    },
  )

  return [webSearch, readPages, findSimilar, answer]
}
