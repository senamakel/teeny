import type { ExaSearch, ExaSearchHit } from '../agent/tools/exa.js'

/** One story as the pipeline sees it: enough to choose from, not enough to quote. */
export interface Story {
  title: string
  url: string
  source: string
  publishedAt?: string
  excerpt: string
  /** Which topic search found it, e.g. "AI". */
  topic: string
}

export interface NewsSource {
  /** Recent stories on a topic, newest the search can find first. */
  search(topic: string, options: { days: number; limit: number }): Promise<Story[]>
}

/** Exa, bounded to the last day or two and to its news index. */
export class ExaNewsSource implements NewsSource {
  constructor(private readonly exa: ExaSearch) {}

  async search(topic: string, options: { days: number; limit: number }): Promise<Story[]> {
    const since = new Date(Date.now() - options.days * 86_400_000).toISOString()
    const { results } = await this.exa
      .searchAndContents(topic, {
        numResults: options.limit,
        type: 'auto',
        category: 'news',
        startPublishedDate: since,
        highlights: { numSentences: 2, highlightsPerUrl: 2 },
      })
      .catch(() => ({ results: [] as ExaSearchHit[] }))
    return results.flatMap((hit) => {
      const title = hit.title?.trim()
      if (!title) return []
      return [
        {
          title,
          url: hit.url,
          source: hostOf(hit.url),
          ...(hit.publishedDate ? { publishedAt: hit.publishedDate } : {}),
          excerpt: (hit.highlights?.join(' … ') ?? hit.text ?? '').replace(/\s+/g, ' ').slice(0, 400).trim(),
          topic,
        },
      ]
    })
  }
}

function hostOf(url: string): string {
  try {
    return new URL(url).hostname.replace(/^www\./, '')
  } catch {
    return 'unknown'
  }
}

/**
 * Words that mark a story as political, whatever else it is about.
 *
 * The model is told to skip politics too, but a filter it cannot argue with
 * is cheaper than a prompt it might, and a story that never reaches the
 * model cannot be chosen by it. Word boundaries, case-insensitive, matched
 * against the title and the excerpt. Deliberately blunt: a good story about
 * a chip export rule is a small loss next to a morning post about a war.
 */
export const POLITICAL_TERMS = [
  'election',
  'elections',
  'electoral',
  'ballot',
  'president',
  'presidential',
  'prime minister',
  'senate',
  'senator',
  'lawmaker',
  'lawmakers',
  'house speaker',
  'policymaker',
  'policymakers',
  'minister',
  'ministry',
  'federal',
  'feds',
  'treasury',
  'justice department',
  'north korea',
  'north korean',
  'congress',
  'congressional',
  'parliament',
  'legislation',
  'legislature',
  'governor',
  'white house',
  'kremlin',
  'republican',
  'republicans',
  'democrat',
  'democrats',
  'gop',
  'labour party',
  'tory',
  'tories',
  'trump',
  'biden',
  'harris',
  'putin',
  'zelensky',
  'netanyahu',
  'xi jinping',
  'modi',
  'war',
  'warfare',
  'invasion',
  'ceasefire',
  'missile',
  'missiles',
  'airstrike',
  'airstrikes',
  'troops',
  'military',
  'gaza',
  'israel',
  'israeli',
  'palestine',
  'palestinian',
  'ukraine',
  'ukrainian',
  'russia',
  'russian',
  'iran',
  'iranian',
  'taiwan',
  'nato',
  'tariff',
  'tariffs',
  'sanction',
  'sanctions',
  'immigration',
  'immigrants',
  'deportation',
  'abortion',
  'protest',
  'protests',
  'protesters',
  'riot',
  'riots',
  'impeach',
  'impeachment',
  'indictment',
  'indicted',
  'lawsuit',
  'supreme court',
  'regulator',
  'regulators',
  'regulation',
  'regulatory',
  'doj',
  'fbi',
  'cia',
  'shooting',
  'terror',
  'terrorist',
  'genocide',
]

const POLITICAL = new RegExp(
  `(?:^|[^a-z])(?:${POLITICAL_TERMS.map((term) => term.trim().replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).join('|')})(?:$|[^a-z])`,
  'i',
)

export function isPolitical(story: Pick<Story, 'title' | 'excerpt'>): boolean {
  return POLITICAL.test(`${story.title} ${story.excerpt}`)
}

/**
 * The morning's reading: every topic searched, the political stories dropped,
 * duplicates by URL folded, newest first within a topic. A search that fails
 * contributes nothing rather than failing the morning.
 */
export async function gatherNews(
  source: NewsSource,
  options: { topics: string[]; days: number; perTopic: number },
): Promise<{ stories: Story[]; dropped: number }> {
  const seen = new Set<string>()
  const stories: Story[] = []
  let dropped = 0
  for (const topic of options.topics) {
    const found = await source
      .search(topic, { days: options.days, limit: options.perTopic })
      .catch(() => [] as Story[])
    for (const story of found) {
      const key = story.url.replace(/[#?].*$/, '').toLowerCase()
      if (seen.has(key)) continue
      seen.add(key)
      if (isPolitical(story)) {
        dropped += 1
        continue
      }
      stories.push(story)
    }
  }
  return { stories, dropped }
}

/** The stories as the model reads them: numbered from one, with the number being the handle. */
export function renderStories(stories: Story[]): string {
  return stories
    .map((story, index) =>
      [
        `[${index + 1}] ${story.title}`,
        `    ${story.source}${story.publishedAt ? `, ${story.publishedAt.slice(0, 10)}` : ''} (${story.topic})`,
        story.excerpt ? `    ${story.excerpt}` : undefined,
      ]
        .filter(Boolean)
        .join('\n'),
    )
    .join('\n')
}
