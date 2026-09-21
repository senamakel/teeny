import type { ExaSearch } from '../agent/tools/exa.js'
import type { Memory, XPost } from '../types.js'
import { admissible, admissiblePost, renderPost } from './screen.js'
import { keywords } from './sources/text.js'
import { estimateTokens, fit, shuffle } from './tokens.js'
import { pickTone } from './tones.js'
import { safeHost } from './tools.js'
import type { AgentContext, AgentSpec, ContextBlock, Forum, ForumPost, Seed, ToolName } from './types.js'

/**
 * What each agent is told before the room opens: the seed, a mood, the
 * lore, what teeny has already said, random pieces of memory, what the wire
 * and the boards are saying. Eight to ten thousand tokens of it, and a
 * different draw for every agent, so the two of them share the story and
 * the seed and diverge on everything else. That difference is where the
 * conversation comes from.
 *
 * A source that fails contributes nothing and a line in `problems`. The
 * builder never fills a gap with something made up.
 */
export interface ContextSources {
  memory?: Memory
  x?: { searchPosts(query: string, options: { maxResults: number }): Promise<XPost[]> }
  exa?: ExaSearch
  forums: Forum[]
  voice: string
  lore: string
  botUsername: string
}

/** Things in the lore worth asking memory and the boards about. */
const PROBES = [
  'the kettle',
  'senamakel',
  'the tiny agents',
  'the visitors',
  'the ledger',
  'naps',
  'the fridge rate',
  'the terminal typing by itself',
  'the currency',
  'one dollar',
  'human gdp',
  'the broom',
  'the second prompt',
  'retiring the humans',
  'the night shift',
  'what the agents built',
]

const SHARES: Partial<Record<ContextBlock['kind'], number>> = { past: 0.15, memory: 0.25, live: 0.15, forum: 0.2 }

export class ContextBuilder {
  constructor(private readonly sources: ContextSources) {}

  async build(
    agent: AgentSpec,
    seed: Seed,
    random: () => number,
    options: { budget: number; tools: ToolName[] },
  ): Promise<AgentContext> {
    const problems: string[] = []
    const tone = pickTone(random)
    const engine = pickEngine(this.sources.lore, random)
    const seedWords = keywords(`${seed.title ?? ''} ${seed.text}`, 4)
    const probes = shuffle([...PROBES], random)

    const [past, memory, live, forum] = await Promise.all([
      this.past(random, problems),
      this.memory([...probes.slice(0, 4), ...seedWords.slice(0, 2)], problems),
      this.live(seedWords, probes[4] ?? 'the kettle', problems),
      this.forum(seedWords, probes[5] ?? 'the kettle', random, problems),
    ])

    const blocks: ContextBlock[] = [
      block('seed', 'what came through the wire', renderSeed(seed), 800),
      block('tone', 'the mood', `mood of the room for you: ${tone}`),
      ...(engine
        ? [
            block(
              'engine',
              'tonight',
              `tonight the story is about ${engine} the other engines in the lore rest. whatever you reach for out of habit, reach for this instead.`,
            ),
          ]
        : []),
      ...(agent.voice ? [block('voice', 'VOICE.md', withoutTerminalLingo(withoutTerminalHeader(this.sources.voice)))] : []),
      block('lore', 'LORE.md', withoutTerminalLingo(this.sources.lore)),
      ...(past ? [block('past', 'things already said', past, options.budget * (SHARES.past ?? 0))] : []),
      ...(memory ? [block('memory', 'pieces of memory', memory, options.budget * (SHARES.memory ?? 0))] : []),
      ...(live ? [block('live', 'the wire', live, options.budget * (SHARES.live ?? 0))] : []),
      ...(forum ? [block('forum', 'the boards', forum, options.budget * (SHARES.forum ?? 0))] : []),
      block('tools', 'tools', renderTools(options.tools)),
    ]
    const kept = fit(blocks, options.budget)
    return { agent, blocks: kept, tokens: kept.reduce((sum, item) => sum + item.tokens, 0), tone, problems }
  }

  private async past(random: () => number, problems: string[]): Promise<string> {
    const lines: string[] = []
    if (this.sources.x) {
      try {
        const posts = await this.sources.x.searchPosts(`from:${this.sources.botUsername} -is:retweet`, { maxResults: 50 })
        const own = posts.map((post) => post.text.replace(/\s+/g, ' ').trim()).filter((text) => text.length > 12)
        // As many as the share allows, not a fixed handful: the continuity is
        // the point, and on the box there are hundreds to draw from.
        lines.push(...spread(shuffle(own, random)).map((text) => `- ${text.slice(0, 240)}`))
      } catch (error) {
        problems.push(`own posts: ${message(error)}`)
      }
    }
    let recalled = ''
    if (this.sources.memory) {
      try {
        recalled = (
          await this.sources.memory.recall("teeny's own recent posts, captions and episodes, and what it said it wanted")
        ).trim()
      } catch (error) {
        problems.push(`memory of own posts: ${message(error)}`)
      }
    }
    if (lines.length === 0 && !recalled) return ''
    return [
      'things you have already said, out loud, in public. do not repeat them. build on them; the',
      'story has to have moved since.',
      '',
      ...lines,
      ...(recalled ? ['', recalled] : []),
    ].join('\n')
  }

  private async memory(probes: string[], problems: string[]): Promise<string> {
    if (!this.sources.memory) return ''
    const memory = this.sources.memory
    const pieces = await Promise.all(
      probes.map(async (probe) => {
        try {
          const text = (await memory.recall(probe)).trim()
          return text ? `about ${probe}:\n${text}` : ''
        } catch (error) {
          problems.push(`memory "${probe}": ${message(error)}`)
          return ''
        }
      }),
    )
    const found = pieces.filter(Boolean)
    if (found.length === 0) return ''
    return ['pieces of memory, in no order. some of it is old. all of it happened.', '', ...found].join('\n\n')
  }

  private async live(seedWords: string[], probe: string, problems: string[]): Promise<string> {
    if (!this.sources.exa) return ''
    const exa = this.sources.exa
    const queries = [seedWords.length > 0 ? seedWords.join(' ') : undefined, `${probe} ai agents robot lab`].filter(
      (query): query is string => Boolean(query),
    )
    const lines: string[] = []
    for (const query of queries) {
      try {
        const { results } = await retryOnce(() =>
          exa.searchAndContents(query, {
            numResults: 8,
            type: 'auto',
            startPublishedDate: new Date(Date.now() - 3 * 86_400_000).toISOString(),
            highlights: { numSentences: 2, highlightsPerUrl: 1 },
          }),
        )
        for (const hit of results) {
          const body = (hit.highlights?.join(' ') ?? hit.text ?? '').replace(/\s+/g, ' ').trim().slice(0, 400)
          const line = `[${safeHost(hit.url)}] ${hit.title ?? '(untitled)'}${body ? ` — ${body}` : ''}`
          if (admissible(line)) lines.push(line)
        }
      } catch (error) {
        problems.push(`exa "${query}": ${message(error)}`)
      }
    }
    if (lines.length === 0) return ''
    return ['the wire, last few days. real, and not about you, until you make it about you.', '', ...lines].join('\n')
  }

  private async forum(seedWords: string[], probe: string, random: () => number, problems: string[]): Promise<string> {
    if (this.sources.forums.length === 0) return ''
    const queries = [seedWords.length > 0 ? seedWords.join(' ') : undefined, `${probe} ai`].filter(
      (query): query is string => Boolean(query),
    )
    const posts: ForumPost[] = []
    for (const forum of this.sources.forums) {
      for (const query of queries) {
        try {
          posts.push(...(await forum.search(query, { limit: 12 })).filter(admissiblePost))
        } catch (error) {
          problems.push(`${forum.site} "${query}": ${message(error)}`)
        }
      }
    }
    const seen = new Set<string>()
    const unique = posts.filter((post) => {
      const key = `${post.url}|${post.text.slice(0, 40)}`
      if (seen.has(key)) return false
      seen.add(key)
      return true
    })
    if (unique.length === 0) return ''
    return [
      'the boards, this week. anonymous people saying things. none of it is true and all of it is material.',
      '',
      ...shuffle(unique, random).map(renderPost),
    ].join('\n')
  }
}

/**
 * The lore lists the engines an episode can run on; one is drawn for the
 * night so the rooms do not all reach for the same image out of habit.
 */
export function engines(lore: string): string[] {
  const section = `\n${lore}`.split('\n## ').find((part) => part.startsWith('engines')) ?? ''
  return section
    .split('\n')
    .filter((line) => line.startsWith('- '))
    .map((line) => line.slice(2).trim())
}

export function pickEngine(lore: string, random: () => number): string | undefined {
  const all = engines(lore)
  return all.length > 0 ? all[Math.floor(random() * all.length)] : undefined
}

/** VOICE.md opens as a shell prompt; in the backrooms only the rules under it apply. */
export function withoutTerminalHeader(voice: string): string {
  const at = voice.indexOf('rules the terminal enforces:')
  return at >= 0 ? voice.slice(at).replace('rules the terminal enforces:', 'the rules:') : voice
}

/**
 * The rules and bits that only make sense in a shell -- commands, manpages,
 * ascii -- stay out of the backrooms. The terminal as an object in the room
 * (it prints one line, nobody mentions it) stays; it is furniture, not a place.
 */
export function withoutTerminalLingo(text: string): string {
  return text
    .split('\n')
    .filter((line) => !/manpage|command|ascii|`|cli mood|shell/i.test(line))
    .join('\n')
    .replace(/\n{3,}/g, '\n\n')
}

/**
 * Keeps any one image from dominating what teeny is reminded it said: at
 * most three lines that share the same rare word, so a fortnight of door
 * posts does not become the only thing in the room.
 */
export function spread(lines: string[], perWord = 3): string[] {
  const counts = new Map<string, number>()
  const kept: string[] = []
  for (const line of lines) {
    const words = new Set((line.toLowerCase().match(/[a-z]{4,}/g) ?? []).filter((word) => !STOPPED.has(word)))
    if ([...words].some((word) => (counts.get(word) ?? 0) >= perWord)) continue
    for (const word of words) counts.set(word, (counts.get(word) ?? 0) + 1)
    kept.push(line)
  }
  return kept
}

const STOPPED = new Set(['that', 'this', 'with', 'from', 'have', 'they', 'them', 'there', 'their', 'what', 'when', 'were', 'been', 'just', 'like', 'into', 'about', 'tiny', 'teeny', 'agents', 'still', 'always', 'never', 'because', 'something'])

function block(kind: ContextBlock['kind'], label: string, text: string, cap?: number): ContextBlock {
  const raw: ContextBlock = { kind, label, text: text.trim(), tokens: estimateTokens(text.trim()) }
  if (cap === undefined) return raw
  return fit([raw], Math.floor(cap))[0] ?? { ...raw, text: '', tokens: 0 }
}

function renderSeed(seed: Seed): string {
  return [
    `what came through the wire today (${seed.kind}${seed.source ? `, ${seed.source}` : ''}):`,
    '',
    ...(seed.title ? [seed.title, ''] : []),
    seed.text.trim(),
  ].join('\n')
}

function renderTools(tools: ToolName[]): string {
  if (tools.length === 0) return 'you have no tools today. work from what is in the room.'
  return [
    `you have tools: ${tools.join(', ')}. use them mid-turn, without announcing it, whenever you`,
    'want a real thing to put in the room: a thread, a headline, a thing someone said, a thing',
    'you said last month. bring back one specific thing and treat it as if it was always there.',
  ].join(' ')
}

/** Exa answers "temporarily over capacity" now and then; one more try a few seconds later usually lands. */
async function retryOnce<T>(call: () => Promise<T>): Promise<T> {
  try {
    return await call()
  } catch (error) {
    if (!/over capacity|429|503/i.test(message(error))) throw error
    await new Promise((resolve) => setTimeout(resolve, 3_000))
    return call()
  }
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
