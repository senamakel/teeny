import { WEIGHTS } from '../critic/algo.js'
import type { XPost, XSearchOptions, XTrend } from '../types.js'
import { isPolitical, type Story } from './news.js'

/**
 * One search over X's recent posts, standing for one corner of the timeline.
 * `share` is how many of the day's posts it is allowed relative to the
 * others: the AI-meme corner gets two for every one the rest get, because a
 * joke people are already making is the surest thing to make a morning post
 * about.
 */
export interface TimelineQuery {
  label: string
  query: string
  topic: string
  share?: number
}

/**
 * What the morning reads off X itself. X's search operators do the first
 * cut: originals only (no reposts, replies or quotes), English, with a
 * picture where the corner is memes. The words are the corner's own
 * vocabulary; a post that says "cooked" about a model is a post people are
 * laughing at, and that is what the pipeline wants to find.
 */
export const TIMELINE_QUERIES: TimelineQuery[] = [
  {
    label: 'ai memes',
    topic: 'ai',
    share: 2,
    query:
      '(chatgpt OR claude OR gemini OR grok OR openai OR anthropic OR llm OR "the model" OR "vibe coding" OR "ai agents") ' +
      '(meme OR lol OR lmao OR cooked OR unhinged OR "so back" OR "it\'s over" OR "absolutely right" OR "skill issue" OR "hallucinated") ' +
      'has:media -has:cashtags -is:retweet -is:reply -is:quote lang:en -airdrop -presale -giveaway -launchpad',
  },
  {
    label: 'ai',
    topic: 'ai',
    query:
      '(openai OR anthropic OR deepmind OR mistral OR "open source model" OR "open-source model" OR "open weights" OR ' +
      '"ai agents" OR agentic OR llm) -has:cashtags -is:retweet -is:reply -is:quote lang:en -airdrop -presale -giveaway',
  },
  {
    label: 'crypto builders',
    topic: 'crypto',
    query:
      '(ethereum OR solana OR onchain OR defi OR "ai agent" OR "agent economy") (shipped OR launched OR "open source" OR ' +
      'builders OR lol OR meme) -has:cashtags -is:retweet -is:reply -is:quote lang:en -airdrop -presale -giveaway -launchpad -whitelist',
  },
]

/**
 * A post with nobody talking under it is not a conversation to walk into,
 * whatever it says. Two replies, or twenty likes, as the ranker counts them.
 */
export const MIN_CONVERSATION = 10

export interface TimelineSource {
  /** What X says is trending right now, or nothing when it will not say. */
  trending(): Promise<XTrend[]>
  /** Recent original posts for one query, as stories, best conversations first. */
  pulse(query: TimelineQuery, options: { hours: number; limit: number }): Promise<Story[]>
}

/**
 * The timeline through the X API: recent search, sorted by X's own
 * relevancy so the first page is the posts people engaged with rather than
 * the last ten written, and the trends endpoint where the tier has it.
 */
export class XTimelineSource implements TimelineSource {
  constructor(
    private readonly x: {
      searchPosts(query: string, options?: XSearchOptions): Promise<XPost[]>
      trends?(): Promise<XTrend[]>
    },
    private readonly now: () => number = Date.now,
  ) {}

  async trending(): Promise<XTrend[]> {
    if (!this.x.trends) return []
    return this.x.trends().catch(() => [])
  }

  async pulse(query: TimelineQuery, options: { hours: number; limit: number }): Promise<Story[]> {
    // X wants end_time at least ten seconds in the past; a minute is safe.
    const end = this.now() - 60_000
    const posts = await this.x
      .searchPosts(query.query, {
        maxResults: options.limit,
        startTime: new Date(end - options.hours * 3_600_000).toISOString(),
        endTime: new Date(end).toISOString(),
        sortOrder: 'relevancy',
      })
      .catch(() => [] as XPost[])
    return posts
      .map((post) => storyOf(post, query))
      .sort((a, b) => conversationScore(b) - conversationScore(a))
  }
}

/**
 * A post that is somebody's promotion rather than a conversation: the
 * words, a contract address, or a cashtag that is not our own. X's
 * `-has:cashtags` catches most of the last in the search; the pattern
 * catches what it lets through.
 */
const SPAM =
  /\b(giveaway|airdrop|presale|pre-sale|whitelist|pump|100x|1000x|dm me|link in bio|position yourself|alpha list|join (?:my|our) (?:telegram|discord))\b|0x[a-f0-9]{40}|\bca:|\$(?!TINY\b)[A-Za-z]{2,10}\b/i

/**
 * A post as a story: the first line is the headline, the whole text the
 * excerpt, the author its source, and the counters carried across so the
 * picker can see how much of a conversation it already is. The URL is the
 * post's own, which is what gets remembered and never what gets posted.
 */
export function storyOf(post: XPost, query: TimelineQuery): Story {
  const text = post.text.replace(/https?:\/\/\S+/g, '').replace(/\s+/g, ' ').trim()
  const firstLine = post.text.split(/\n/).map((line) => line.replace(/https?:\/\/\S+/g, '').trim()).find(Boolean) ?? text
  const title = (firstLine.length > 140 ? `${firstLine.slice(0, 137).trimEnd()}…` : firstLine) || '(a picture)'
  const media = post.media?.map((item) => item.altText).find(Boolean)
  return {
    title,
    url: `https://x.com/${post.authorUsername ?? 'i'}/status/${post.id}`,
    source: post.authorUsername ? `@${post.authorUsername} on X` : 'a post on X',
    ...(post.createdAt ? { publishedAt: post.createdAt } : {}),
    excerpt: text.slice(0, 400),
    topic: query.label,
    kind: 'x',
    ...(post.metrics ? { metrics: post.metrics } : {}),
    ...(media ? { media } : {}),
  }
}

/**
 * How much conversation a post has already earned, valued as the ranker
 * values it: a reply or a quote is worth ten likes, a repost two. This is
 * the order the picker sees the posts in, so the ones the algorithm itself
 * would rank first are first.
 */
export function conversationScore(story: Story): number {
  const m = story.metrics
  if (!m) return 0
  return m.replies * WEIGHTS.reply + m.quotes * WEIGHTS.quote + m.reposts * WEIGHTS.repost + m.likes * WEIGHTS.like
}

/**
 * The morning's read of the timeline: every corner searched, promotions and
 * politics dropped, duplicates folded, each corner's share honoured. Fails
 * soft the way the news does: a search that cannot be made contributes
 * nothing, and a morning with no timeline is a morning with only the news.
 */
export async function gatherTimeline(
  source: TimelineSource,
  options: { queries: TimelineQuery[]; hours: number; perQuery: number },
): Promise<{ posts: Story[]; dropped: number; quiet: number; trends: XTrend[] }> {
  if (options.perQuery <= 0) return { posts: [], dropped: 0, quiet: 0, trends: [] }
  const trends = await source.trending().catch(() => [] as XTrend[])
  const seen = new Set<string>()
  const posts: Story[] = []
  let dropped = 0
  let quiet = 0
  for (const query of options.queries) {
    const limit = options.perQuery * (query.share ?? 1)
    const found = await source.pulse(query, { hours: options.hours, limit }).catch(() => [] as Story[])
    for (const story of found.slice(0, limit)) {
      if (seen.has(story.url)) continue
      seen.add(story.url)
      if (isPolitical(story) || SPAM.test(story.excerpt)) {
        dropped += 1
        continue
      }
      if (conversationScore(story) < MIN_CONVERSATION) {
        quiet += 1
        continue
      }
      posts.push(story)
    }
  }
  return { posts, dropped, quiet, trends }
}

/** The trends as the model reads them, most posts first when X counts them. */
export function renderTrends(trends: XTrend[]): string {
  return [...trends]
    .sort((a, b) => (b.posts ?? 0) - (a.posts ?? 0))
    .map((trend) => `- ${trend.name}${trend.posts ? ` (${trend.posts.toLocaleString('en-US')} posts)` : ''}`)
    .join('\n')
}

/**
 * Queries from configuration: `label=query` entries separated by `;`, or
 * the built-in corners when the string is empty. The topic is the label.
 */
export function parseTimelineQueries(value: string | undefined): TimelineQuery[] {
  if (!value?.trim()) return TIMELINE_QUERIES
  return value
    .split(';')
    .map((entry) => entry.trim())
    .filter(Boolean)
    .map((entry) => {
      const at = entry.indexOf('=')
      const label = at > 0 ? entry.slice(0, at).trim() : entry
      const query = at > 0 ? entry.slice(at + 1).trim() : entry
      return { label, query, topic: label }
    })
}
