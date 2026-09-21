import { isTrustedHandle } from '../agent/trusted.js'
import type { XPost, XSearchOptions } from '../types.js'
import type { QuotedPost } from './store.js'

/** Where a candidate came from: the ticker search, or the account's mentions. */
export type CandidateSource = 'ticker' | 'mention'

export interface Candidate {
  post: XPost
  source: CandidateSource
  /** The author is one of the lab's handles. */
  trusted: boolean
  /** How much of a conversation the post already is, as the ranker counts it. */
  score: number
}

export interface CandidateRules {
  /** The account's own handle; its own posts are never candidates. */
  botUsername: string
  /** The lab's handles, which are flagged rather than filtered. */
  trustedHandles: string[]
  /** The account's own contract addresses; any other address in a post is somebody else's shill. */
  ownTokenAddresses: string[]
  /** The account's own ticker, without the `$`. Any other cashtag is somebody else's. */
  ownSymbol: string
  /** What the account already quoted. */
  quoted: QuotedPost[]
  /** A post older than this is not "recent" any more. */
  maxAgeMs: number
  /** An author quoted inside this window is not quoted again. */
  authorCooldownMs: number
  /** The content filter's inbound lexicon; true when the text is clean. */
  screen?: (text: string) => boolean
  now: number
}

/**
 * A base58 run the length of a Solana address, or an EVM one. Somebody
 * posting a contract address that is not ours is selling something, and
 * the account never quotes that -- a quote is an endorsement whatever the
 * text over it says.
 */
const ADDRESS = /\b(?:0x[a-fA-F0-9]{40}|[1-9A-HJ-NP-Za-km-z]{32,44})\b/g

/** Words that make a post a promotion rather than somebody talking. */
const PROMOTION =
  /\b(giveaway|airdrop|presale|pre-sale|whitelist|launchpad|dm me|link in bio|position yourself|alpha list|join (?:my|our) (?:telegram|discord)|claim (?:your|their) (?:share|allocation|reward))\b/i

/**
 * Why a post is not quoted, or nothing when it may be. The rules are the
 * operator's, in order of how cheap they are to state: it is ours, it has
 * been done, it is old, it is selling something else, it says nothing.
 */
export function reasonToSkip(post: XPost, rules: CandidateRules): string | undefined {
  const handle = rules.botUsername.replace(/^@/, '').toLowerCase()
  const author = post.authorUsername?.replace(/^@/, '').toLowerCase()
  if (author === handle) return 'our own post'
  if (rules.quoted.some((entry) => entry.postId === post.id)) return 'already quoted'
  if (author) {
    const recent = rules.quoted.find(
      (entry) =>
        entry.authorUsername?.toLowerCase() === author &&
        rules.now - Date.parse(entry.quotedAt) < rules.authorCooldownMs,
    )
    if (recent) return `@${post.authorUsername} was quoted on ${recent.quotedAt.slice(0, 10)}`
  }
  if (post.createdAt) {
    const age = rules.now - Date.parse(post.createdAt)
    if (Number.isFinite(age) && age > rules.maxAgeMs) return 'older than the window'
  }

  const text = post.text
  const own = new Set(rules.ownTokenAddresses.map((address) => address.toLowerCase()))
  for (const match of text.match(ADDRESS) ?? []) {
    if (!own.has(match.toLowerCase())) return 'carries a contract address that is not ours'
  }
  const cashtags = [...text.matchAll(/\$([A-Za-z][A-Za-z0-9]{1,9})\b/g)].map((match) => match[1]!.toUpperCase())
  const foreign = cashtags.find((symbol) => symbol !== rules.ownSymbol.toUpperCase())
  if (foreign) return `names another token ($${foreign})`
  if (PROMOTION.test(text)) return 'reads as promotion'
  if (rules.screen && !rules.screen(text)) return 'the content filter'

  const words = text
    .replace(/https?:\/\/\S+/g, ' ')
    .replace(/@\w+/g, ' ')
    .replace(ADDRESS, ' ')
    .replace(/\bsolana:\s*/gi, ' ')
    .split(/\s+/)
    .filter((word) => /[A-Za-z0-9]/.test(word))
  if (words.length < 3) return 'says nothing beyond the ticker'
  return undefined
}

/** Likes, replies twice, reposts and quotes three times: what the ranker counts as a conversation. */
export function conversationScore(post: XPost): number {
  const metrics = post.metrics
  if (!metrics) return 0
  return metrics.likes + 2 * metrics.replies + 3 * (metrics.reposts + metrics.quotes)
}

export interface CandidateSearch {
  searchPosts(query: string, options?: XSearchOptions): Promise<XPost[]>
}

export interface FindOptions extends CandidateRules {
  /** The operator's ticker query; retweets and the account's own posts are cut here. */
  query: string
  /** How many posts each of the two searches may read. X floors this at 10. */
  maxResults: number
  /** How many candidates are handed to the picker, best conversations first. */
  keep: number
  /** How many of one author's posts may be in the list. Two by default: a list that is one person seven times is that person's list. */
  perAuthor?: number
  logger?: { info: (object: unknown, message?: string) => void }
}

export interface Found {
  candidates: Candidate[]
  /** Every post the searches returned that was not kept, and why. */
  skipped: Array<{ id: string; author?: string; reason: string }>
  searched: number
}

/**
 * Two searches, one list.
 *
 * The ticker search is the operator's query -- the cashtag or the contract
 * address, which people paste as `solana:<address>` -- with reposts cut by
 * X and replies kept, because a bull answering somebody else's thread with
 * the ticker is still a bull. The mention search is everybody who talked to
 * the account. Both are bounded to the window and read by relevancy, so the
 * first page is what people engaged with rather than the last fifty
 * written. Each post is then read against the rules, and what survives is
 * sorted with the lab's own posts first and the busiest conversation next.
 */
export async function findCandidates(x: CandidateSearch, options: FindOptions): Promise<Found> {
  const handle = options.botUsername.replace(/^@/, '')
  // X wants end_time at least ten seconds in the past; a minute is safe.
  const end = options.now - 60_000
  const window = {
    maxResults: options.maxResults,
    startTime: new Date(end - options.maxAgeMs).toISOString(),
    endTime: new Date(end).toISOString(),
    sortOrder: 'relevancy' as const,
  }
  const searches: Array<[CandidateSource, string]> = [
    ['ticker', `(${options.query}) -is:retweet -from:${handle}`],
    ['mention', `(@${handle} OR to:${handle}) -from:${handle} -is:retweet`],
  ]
  const seen = new Set<string>()
  const candidates: Candidate[] = []
  const skipped: Found['skipped'] = []
  let searched = 0
  for (const [source, query] of searches) {
    const posts = await x.searchPosts(query, window).catch((error: unknown) => {
      options.logger?.info({ source, err: error }, 'quote: a search failed')
      return [] as XPost[]
    })
    searched += posts.length
    for (const post of posts) {
      if (seen.has(post.id)) continue
      seen.add(post.id)
      const reason = reasonToSkip(post, options)
      if (reason) {
        skipped.push({ id: post.id, ...(post.authorUsername ? { author: post.authorUsername } : {}), reason })
        continue
      }
      candidates.push({
        post,
        source,
        trusted: isTrustedHandle(post.authorUsername, options.trustedHandles),
        score: conversationScore(post),
      })
    }
  }
  candidates.sort((a, b) => Number(b.trusted) - Number(a.trusted) || b.score - a.score)
  const perAuthor = options.perAuthor ?? 2
  const counts = new Map<string, number>()
  const kept: Candidate[] = []
  for (const candidate of candidates) {
    const author = candidate.post.authorUsername?.toLowerCase() ?? candidate.post.id
    const count = counts.get(author) ?? 0
    if (count >= perAuthor) {
      skipped.push({ id: candidate.post.id, ...(candidate.post.authorUsername ? { author: candidate.post.authorUsername } : {}), reason: 'the author already has two in the list' })
      continue
    }
    counts.set(author, count + 1)
    kept.push(candidate)
  }
  return { candidates: kept.slice(0, options.keep), skipped, searched }
}
