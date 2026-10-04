import type { OwnPost, PostLedger } from '../cadence/ledger.js'
import { summariseRecall } from '../journal/journal.js'
import type { Exchange } from '../queue/mentions.js'
import type { Memory, Person } from '../types.js'

/** The question that stood in for the ledger before there was one to read. */
export const OWN_ACTIVITY_QUERY =
  'What has this account itself posted recently? Include its daily chart posts, the ' +
  'market reads it published, and anything it has said publicly in the last few days.'

export const OWN_ACTIVITY_HEADING = 'WHAT YOU YOURSELF HAVE POSTED RECENTLY:'

/** What a turn is told before it is drafted. */
export interface TurnMemory {
  /** The text the drafter reads under "MEMORY ALREADY RECALLED". */
  text: string
  /** A few lines for the journal: what was actually recalled, not a paraphrase. */
  recalled: string[]
  /** Sources that failed or timed out this turn, by name. Empty is a whole turn. */
  degraded: string[]
}

export interface TurnContextOptions {
  memory: Pick<Memory, 'recall'>
  /** The account's own timeline, read exactly. Absent, memory is asked instead. */
  ownPosts?: Pick<PostLedger, 'since'>
  /** The last answered exchanges with an X user, read exactly. */
  exchanges?: (authorId: string, limit: number, options: { excludePostId?: string }) => Promise<Exchange[]>
  /** The same human on two channels, as pairs of person ids. */
  links?: ReadonlyArray<readonly [string, string]>
  limits?: { exchanges?: number; ownPostsHours?: number; ownPostsLimit?: number }
  now?: () => number
  logger?: { warn: (object: unknown, message?: string) => void }
}

const DEFAULT_LIMITS = { exchanges: 6, ownPostsHours: 72, ownPostsLimit: 8 }

/** Wall-clock time the way the drafter reads it: to the minute, in UTC. */
function stamp(iso: string): string {
  return `${iso.slice(0, 16).replace('T', ' ')} UTC`
}

function clip(text: string, max: number): string {
  const flat = text.replace(/\s+/g, ' ').trim()
  return flat.length > max ? `${flat.slice(0, max - 1)}…` : flat
}

/**
 * Everything a turn starts with, gathered in one place.
 *
 * Before this, each harness asked memory two questions and took whatever came
 * back, including nothing: a recall that timed out read exactly like a person
 * the account had never met, and the drafter said so. Three things changed:
 *
 * - What can be read exactly is read exactly. The account's own posts are in
 *   the post ledger and its answers to a person are in the mention queue;
 *   asking a semantic index for either is asking it to guess at a list that
 *   already exists. Asked "what has this account posted recently", memory
 *   answered with month-old replies to other people.
 * - Every source fails on its own. Memory being down costs the turn its
 *   recalled context, not the ledger or the exchange history, and the reverse.
 * - A failure is said out loud. The drafter is told memory was unavailable, so
 *   it does not claim to remember nothing, and the journal records which
 *   source dropped out.
 */
export class TurnContext {
  private readonly limits: typeof DEFAULT_LIMITS

  constructor(private readonly options: TurnContextOptions) {
    this.limits = { ...DEFAULT_LIMITS, ...definedOnly(options.limits ?? {}) }
  }

  /** The person with every id they are known by on other channels. */
  linked(person: Person): Person {
    const aliases = new Set(person.aliases ?? [])
    for (const [a, b] of this.options.links ?? []) {
      if (a === person.id) aliases.add(b)
      if (b === person.id) aliases.add(a)
    }
    return aliases.size > 0 ? { ...person, aliases: [...aliases] } : person
  }

  async gather(input: {
    query: string
    person?: Person
    /** The X user id, when the turn is a mention: reads the exact exchange history. */
    authorId?: string
    /** The post being answered, kept out of its own history. */
    postId?: string
  }): Promise<TurnMemory> {
    const person = input.person ? this.linked(input.person) : undefined
    const degraded: string[] = []
    const failed = (source: string) => (error: unknown) => {
      degraded.push(source)
      this.options.logger?.warn({ err: error, source }, `turn context: ${source} unavailable`)
      return undefined
    }

    const [exchanges, recalled, own] = await Promise.all([
      this.exchangesWith(input.authorId, input.postId).catch(failed('exchanges')),
      Promise.resolve(this.options.memory.recall(input.query, person ? { person } : {})).catch(failed('memory')),
      this.ownActivity().catch(failed('own posts')),
    ])

    const sections: string[] = []
    const handle = person?.username ? `@${person.username}` : 'this person'
    if (exchanges && exchanges.length > 0) {
      sections.push(
        [
          `YOUR LAST ${exchanges.length} EXCHANGES WITH ${handle}, newest first, exactly as they happened:`,
          ...exchanges.map(
            (exchange) => `[${stamp(exchange.at)}] ${handle}: ${clip(exchange.incoming, 280)}\n  you: ${clip(exchange.reply, 280)}`,
          ),
        ].join('\n'),
      )
    }
    if (recalled === undefined) {
      sections.push(
        'MEMORY WAS UNAVAILABLE THIS TURN. Nothing above or below says you have never met this ' +
          'person or never discussed this; you simply could not look. Do not claim either way.',
      )
    } else if (recalled.trim()) {
      sections.push(recalled.trim())
    }
    if (own?.text.trim()) sections.push(`${OWN_ACTIVITY_HEADING}\n${own.text.trim()}`)

    return {
      text: sections.join('\n\n'),
      // Deduplicated: memory often answers both questions with the same
      // general context when it knows little about the person.
      recalled: [
        ...new Set([
          ...(exchanges && exchanges.length > 0 ? [`${exchanges.length} earlier exchanges with ${handle}`] : []),
          ...summariseRecall(recalled ?? '', 3),
          ...summariseRecall(own?.text ?? '', 2),
          ...degraded.map((source) => `(${source} unavailable)`),
        ]),
      ],
      degraded,
    }
  }

  private async exchangesWith(authorId: string | undefined, postId: string | undefined): Promise<Exchange[]> {
    if (!authorId || !this.options.exchanges || this.limits.exchanges <= 0) return []
    return this.options.exchanges(authorId, this.limits.exchanges, postId ? { excludePostId: postId } : {})
  }

  /**
   * The ledger when there is one, because it is the list itself; memory when
   * there is not, which is how every turn read it before.
   */
  private async ownActivity(): Promise<{ text: string } | undefined> {
    if (this.limits.ownPostsLimit <= 0 || this.limits.ownPostsHours <= 0) return undefined
    if (!this.options.ownPosts) {
      return { text: await this.options.memory.recall(OWN_ACTIVITY_QUERY) }
    }
    const now = (this.options.now ?? Date.now)()
    const since = new Date(now - this.limits.ownPostsHours * 3_600_000).toISOString()
    const posts = await this.options.ownPosts.since(since, this.limits.ownPostsLimit)
    return { text: posts.filter((post) => post.text?.trim()).map(renderOwnPost).join('\n') }
  }
}

function renderOwnPost(post: OwnPost): string {
  return `[${stamp(post.postedAt)} · ${post.kind}] ${clip(post.text ?? '', 400)}`
}

function definedOnly<T extends object>(value: T): Partial<T> {
  return Object.fromEntries(Object.entries(value).filter(([, entry]) => entry !== undefined)) as Partial<T>
}
