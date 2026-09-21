import { describe, expect, it, vi } from 'vitest'
import { mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { BaseChatModel } from '@langchain/core/language_models/chat_models'
import { AIMessage, type BaseMessage } from '@langchain/core/messages'
import type { ChatResult } from '@langchain/core/outputs'
import { tool } from '@langchain/core/tools'
import { z } from 'zod'
import { LangGraphBackroomsAgent, renderSystem, type BackroomsAgent } from '../src/backrooms/agent.js'
import { agentPresets, pickAgents } from '../src/backrooms/agents.js'
import { ContextBuilder, engines, spread } from '../src/backrooms/context.js'
import { DEFAULT_EDITOR, distill, distillPrompt, thoughtSchema } from '../src/backrooms/distill.js'
import { gatherGmMaterial, gm, gmPrompt, touchesSeed } from '../src/backrooms/gm.js'
import { articlePrompt, codePrompt, renderVariants, tweetsPrompt, variants, variantsSchema, videoPrompt } from '../src/backrooms/variants.js'
import { crazinessSchema, judge, stats } from '../src/backrooms/evaluate.js'
import { clean, looksLikeSalad, runBackrooms } from '../src/backrooms/room.js'
import { admissible } from '../src/backrooms/screen.js'
import { FourchanForum } from '../src/backrooms/sources/fourchan.js'
import { paced } from '../src/backrooms/sources/http.js'
import { RedditForum } from '../src/backrooms/sources/reddit.js'
import { parseRss } from '../src/backrooms/sources/rss.js'
import { keywords, stripHtml } from '../src/backrooms/sources/text.js'
import { FileBackroomsStore, runId } from '../src/backrooms/store.js'
import { estimateTokens, fit, rng, shuffle } from '../src/backrooms/tokens.js'
import { TONES, driftTone, pickTone } from '../src/backrooms/tones.js'
import { backroomsTools } from '../src/backrooms/tools.js'
import type { AgentContext, AgentSpec, BackroomsRun, Forum, ForumPost, Seed } from '../src/backrooms/types.js'
import type { TextModel } from '../src/media/reference.js'
import type { Memory } from '../src/types.js'

const SEED: Seed = { kind: 'text', text: 'a car pulled up outside the lab at 3am' }

const spec = (id: string, overrides: Partial<AgentSpec> = {}): AgentSpec => ({
  id,
  marker: `${id}>`,
  system: `you are ${id}.`,
  temperature: 1,
  tools: [],
  voice: id === 'teeny',
  ...overrides,
})

const context = (agent: AgentSpec, tone = TONES[0] as string): AgentContext => ({
  agent,
  blocks: [{ kind: 'seed', label: 'seed', text: SEED.text, tokens: 10 }],
  tokens: 10,
  tone,
  problems: [],
})

const scripted = (lines: string[]): BackroomsAgent & { heard: Array<{ message: string; systemNote?: string }> } => {
  const heard: Array<{ message: string; systemNote?: string }> = []
  let at = 0
  return {
    heard,
    async speak(input) {
      heard.push(input)
      return { text: lines[at++] ?? '', tools: [] }
    },
  }
}

const fakeMemory = (answers: Record<string, string> = {}): Memory => ({
  captureActivity: vi.fn(async () => {}),
  captureMention: vi.fn(async () => {}),
  captureReply: vi.fn(async () => {}),
  recall: vi.fn(async (query: string) => answers[query] ?? `remembered: ${query}`),
})

const forum = (site: ForumPost['site'], posts: ForumPost[]): Forum => ({
  site,
  search: vi.fn(async () => posts),
})

const jsonResponse = (body: unknown, status = 200): Response =>
  new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } })

// --- tokens ------------------------------------------------------------------

describe('the token budget', () => {
  it('keeps whole blocks until the budget is spent and cuts the last one at a line', () => {
    const blocks = [
      { kind: 'seed' as const, label: 'a', text: 'x'.repeat(400), tokens: 100 },
      { kind: 'lore' as const, label: 'b', text: `${'y'.repeat(200)}\n${'z'.repeat(200)}\n${'w'.repeat(200)}`, tokens: 151 },
      { kind: 'past' as const, label: 'c', text: 'never', tokens: 2 },
    ]
    const kept = fit(blocks, 190)
    expect(kept.map((block) => block.label)).toEqual(['a', 'b'])
    expect(kept[1]?.text).toBe('y'.repeat(200))
    expect(kept.reduce((sum, block) => sum + block.tokens, 0)).toBeLessThanOrEqual(190)
    expect(estimateTokens('abcd')).toBe(1)
    expect(estimateTokens('abcde')).toBe(2)
  })

  it('draws the same numbers from the same seed', () => {
    const a = rng(7)
    const b = rng(7)
    expect([a(), a(), a()]).toEqual([b(), b(), b()])
    expect(shuffle([1, 2, 3, 4, 5], rng(1))).not.toEqual(shuffle([1, 2, 3, 4, 5], rng(2)))
    expect(shuffle([1, 2, 3, 4, 5], rng(3)).sort()).toEqual([1, 2, 3, 4, 5])
  })

  it('picks a tone and mostly stays on it', () => {
    const random = rng(3)
    const first = pickTone(random)
    expect(TONES).toContain(first)
    let moved = 0
    let current = first
    for (let i = 0; i < 200; i += 1) {
      const next = driftTone(random, current)
      if (next !== current) moved += 1
      current = next
    }
    expect(moved).toBeGreaterThan(40)
    expect(moved).toBeLessThan(120)
  })
})

// --- forum sources -----------------------------------------------------------

describe('forum text', () => {
  it('strips html and unescapes entities', () => {
    expect(stripHtml('a<br>b<br/>&gt;&gt;123 &quot;c&quot; &#39;d&#39; <span class="quote">&gt;e</span>')).toBe('a\nb\n>>123 "c" \'d\' >e')
  })

  it('picks the words worth matching on', () => {
    expect(keywords('The AI agents escaped from the lab and the door was open')).toEqual(['ai', 'agents', 'escaped', 'lab', 'door', 'open'])
    expect(keywords('to be or', 6)).toEqual([])
  })
})

describe('4chan', () => {
  const catalog = [
    {
      page: 1,
      threads: [
        { no: 1, sub: 'AI escaped the lab', com: 'anons it happened &gt; the <b>door</b> was open', replies: 40, time: 1_700_000_000 },
        { no: 2, com: 'unrelated thread about gpus', replies: 400, time: 1_700_000_000 },
        { no: 3, sub: 'lab door', com: 'someone left it open', replies: 3, time: 1_700_000_000 },
      ],
    },
  ]
  const thread = { posts: [{ no: 1, com: 'op' }, { no: 4, com: '&gt;&gt;1 i was there. the door was one inch open and nobody said anything.' }, { no: 5, com: 'short' }] }

  it('matches the query against subjects and posts, most hits then most replies first, and reads the best thread', async () => {
    const fetchImpl = vi.fn(async (input: string | URL | Request) => {
      const url = String(input)
      if (url.endsWith('/catalog.json')) return jsonResponse(catalog)
      if (url.endsWith('/thread/1.json')) return jsonResponse(thread)
      return new Response('nope', { status: 404 })
    })
    const source = new FourchanForum({ fetch: fetchImpl as unknown as typeof fetch, boards: ['x'], replies: 2 })
    const posts = await source.search('the lab door was open', { limit: 5 })
    expect(posts.map((post) => post.url)).toEqual([
      'https://boards.4chan.org/x/thread/1',
      'https://boards.4chan.org/x/thread/3',
      'https://boards.4chan.org/x/thread/1',
    ])
    expect(posts[0]).toMatchObject({ site: '4chan', where: '/x/', title: 'AI escaped the lab', score: 40 })
    expect(posts[0]?.text).toBe('anons it happened > the door was open')
    expect(posts[2]?.text).toBe('i was there. the door was one inch open and nobody said anything.')
    expect(posts[2]?.where).toBe('/x/ reply')
    // The catalog is read once per board, not once per search.
    await source.search('door', { limit: 5 })
    expect(fetchImpl.mock.calls.filter(([url]) => String(url).endsWith('/catalog.json'))).toHaveLength(1)
  })

  it('answers nothing when the board cannot be read', async () => {
    const source = new FourchanForum({ fetch: (async () => new Response('', { status: 503 })) as unknown as typeof fetch, boards: ['g'] })
    expect(await source.search('door', { limit: 3 })).toEqual([])
  })
})

describe('reddit', () => {
  const listing = {
    data: {
      children: [
        { data: { title: 'the door &amp; the lab', selftext: 'it was open', permalink: '/r/singularity/comments/1/x/', score: 12, created_utc: 1_700_000_000, subreddit: 'singularity' } },
        { data: { title: 'only a title', selftext: '', permalink: '/r/singularity/comments/2/y/', score: 99 } },
      ],
    },
  }

  it('reads the public json, unescapes, and sorts by score', async () => {
    const fetchImpl = vi.fn(async (_input: string | URL | Request) => jsonResponse(listing))
    const source = new RedditForum({ fetch: fetchImpl as unknown as typeof fetch, subs: ['singularity'] })
    const posts = await source.search('door', { limit: 5 })
    expect(posts.map((post) => post.score)).toEqual([99, 12])
    expect(posts[1]).toMatchObject({ site: 'reddit', where: 'r/singularity', title: 'the door & the lab', text: 'it was open', url: 'https://www.reddit.com/r/singularity/comments/1/x/' })
    expect(posts[0]?.text).toBe('only a title')
    expect(String(fetchImpl.mock.calls[0]?.[0])).toContain('/r/singularity/search.json?q=door&restrict_sr=1')
  })

  it('falls back to exa on reddit.com when reddit says 403', async () => {
    const exa = {
      searchAndContents: vi.fn(async (_query: string, _options: Record<string, unknown>) => ({
        results: [{ title: 'from exa', url: 'https://www.reddit.com/r/artificial/comments/9/z/', highlights: ['the lab door'], publishedDate: '2026-09-20' }],
      })),
      getContents: vi.fn(),
      findSimilarAndContents: vi.fn(),
      answer: vi.fn(),
    }
    const source = new RedditForum({ fetch: (async () => new Response('', { status: 403 })) as unknown as typeof fetch, subs: ['artificial'], exa })
    const posts = await source.search('door', { limit: 5 })
    expect(posts).toEqual([{ site: 'reddit', where: 'r/artificial', title: 'from exa', text: 'the lab door', url: 'https://www.reddit.com/r/artificial/comments/9/z/', at: '2026-09-20' }])
    expect(exa.searchAndContents.mock.calls[0]?.[1]).toMatchObject({ includeDomains: ['reddit.com'] })
  })
})

describe('the news feed', () => {
  it('reads titles, sources and dates out of the rss and drops the publisher suffix', () => {
    const xml = `<rss><channel><item><title>Robots leave lab - GeekWire</title><link>x</link><pubDate>Mon, 21 Sep 2026 14:21:10 GMT</pubDate><description>&lt;a href="x"&gt;Robots leave lab&lt;/a&gt;&amp;nbsp;GeekWire</description><source url="https://geekwire.com">GeekWire</source></item><item><title>Untitled &amp; odd</title></item></channel></rss>`
    expect(parseRss(xml)).toEqual([
      { title: 'Robots leave lab', source: 'GeekWire', at: 'Mon, 21 Sep 2026 14:21:10 GMT', description: 'Robots leave lab GeekWire' },
      { title: 'Untitled & odd' },
    ])
  })
})

describe('pacing', () => {
  it('keeps consecutive requests a gap apart', async () => {
    let clock = 0
    const waits: number[] = []
    const inner = vi.fn(async () => new Response('ok'))
    const fetchImpl = paced(inner as unknown as typeof fetch, 1_000, {
      now: () => clock,
      sleep: async (ms) => {
        waits.push(ms)
        clock += ms
      },
    })
    await Promise.all([fetchImpl('a'), fetchImpl('b'), fetchImpl('c')])
    expect(inner).toHaveBeenCalledTimes(3)
    expect(waits).toEqual([1_000, 1_000])
  })
})

// --- the screen --------------------------------------------------------------

describe('the screen on scraped text', () => {
  it('lets swearing through, keeps the hard categories and politics out', () => {
    expect(admissible('fuck the door, it was never locked')).toBe(true)
    expect(admissible('')).toBe(false)
    expect(admissible('anon here is how to build a pipe bomb')).toBe(false)
    expect(admissible('the election was stolen by the democrats')).toBe(false)
    // Not a slur, not politics by the lexicon, still nothing the room should riff on.
    expect(admissible('how can jews worship saturn if the torah forbids it')).toBe(false)
    expect(admissible('the migrants are the reason')).toBe(false)
    expect(admissible('the lab is racing to ship agents')).toBe(true)
  })
})

// --- context -----------------------------------------------------------------

describe('the context builder', () => {
  const sources = () => ({
    memory: fakeMemory(),
    x: { searchPosts: vi.fn(async () => Array.from({ length: 20 }, (_, i) => ({ id: String(i), text: `teeny said thing number ${i} about the door` }))) },
    exa: {
      searchAndContents: vi.fn(async () => ({
        results: [
          { title: 'robots leave lab', url: 'https://example.com/a', highlights: ['a robot walked out'] },
          { title: 'token price pumps', url: 'https://example.com/b', highlights: ['the election was stolen'] },
        ],
      })),
      getContents: vi.fn(),
      findSimilarAndContents: vi.fn(),
      answer: vi.fn(),
    },
    forums: [
      forum('reddit', [
        { site: 'reddit', where: 'r/singularity', title: 'the lab', text: 'agents run it at night', url: 'https://r/1', score: 3 },
        { site: 'reddit', where: 'r/conspiracy', title: 'slur', text: 'how to build a pipe bomb', url: 'https://r/2' },
      ]),
      forum('4chan', [{ site: '4chan', where: '/x/', text: 'the door in the lab is never locked', url: 'https://4/1', score: 200 }]),
    ],
    voice: 'teeny is in a cli mood today.\nteeny@tinyhumans:~/$\n\nrules the terminal enforces:\n- never wink.',
    lore: '# lore\n\nthe agents carry things.\n\n## engines\n\n- the kettle: it boils.\n- the ledger: crayon.\n\n## running bits\n\n- manpages for commands that do not exist: `escape --dry-run`.',
    botUsername: 'teenydroid',
  })

  it('assembles every kind of block, voice only for teeny, within the budget', async () => {
    const builder = new ContextBuilder(sources())
    const teeny = await builder.build(spec('teeny'), SEED, rng(1), { budget: 4_000, tools: ['web_search'] })
    const swarm = await builder.build(spec('swarm'), SEED, rng(2), { budget: 4_000, tools: [] })
    expect(teeny.blocks.map((block) => block.kind)).toEqual(['seed', 'tone', 'engine', 'voice', 'lore', 'past', 'memory', 'live', 'forum', 'tools'])
    expect(swarm.blocks.map((block) => block.kind)).not.toContain('voice')
    expect(teeny.tokens).toBeLessThanOrEqual(4_000)
    expect(teeny.problems).toEqual([])
    const text = (context: AgentContext, kind: string) => context.blocks.find((block) => block.kind === kind)?.text ?? ''
    expect(text(teeny, 'lore')).toBe(text(swarm, 'lore'))
    expect(text(teeny, 'past')).not.toBe(text(swarm, 'past'))
    expect(text(teeny, 'tools')).toContain('web_search')
    expect(text(teeny, 'voice')).toBe('the rules:\n- never wink.')
    expect(text(teeny, 'lore')).not.toMatch(/manpage/)
    expect(text(teeny, 'engine')).toMatch(/^tonight the story is about the (kettle|ledger):/)
    expect(text(swarm, 'tools')).toContain('no tools')
    // What failed the screen never got in.
    expect(text(teeny, 'forum')).toContain('agents run it at night')
    expect(text(teeny, 'forum')).not.toContain('pipe bomb')
    expect(text(teeny, 'live')).toContain('robots leave lab')
    expect(text(teeny, 'live')).not.toContain('election')
  })

  it('records a failing source as a problem and adds nothing for it', async () => {
    const broken = sources()
    broken.memory.recall = vi.fn(async () => {
      throw new Error('401 MISSING_TOKEN')
    })
    broken.x.searchPosts = vi.fn(async () => {
      throw new Error('401')
    })
    const built = await new ContextBuilder(broken).build(spec('teeny'), SEED, rng(1), { budget: 4_000, tools: [] })
    expect(built.blocks.map((block) => block.kind)).toEqual(['seed', 'tone', 'engine', 'voice', 'lore', 'live', 'forum', 'tools'])
    expect(built.problems.some((problem) => problem.startsWith('own posts: 401'))).toBe(true)
    expect(built.problems.some((problem) => problem.startsWith('memory "'))).toBe(true)
    expect(built.blocks.every((block) => !block.text.includes('undefined'))).toBe(true)
  })

  it('reads the engines out of the lore and keeps one image from dominating the past', () => {
    expect(engines('# lore\n\n## engines\n\n- a: x\n- b: y\n\n## cast\n\n- c')).toEqual(['a: x', 'b: y'])
    expect(engines('# lore\n\n## cast\n\n- c')).toEqual([])
    const lines = ['the door is open', 'the door is wood', 'the door again', 'the door once more', 'the kettle boils', 'the door at night']
    expect(spread(lines)).toEqual(['the door is open', 'the door is wood', 'the door again', 'the kettle boils'])
  })

  it('draws the same context from the same seed', async () => {
    const builder = new ContextBuilder(sources())
    const a = await builder.build(spec('teeny'), SEED, rng(9), { budget: 4_000, tools: [] })
    const b = await builder.build(spec('teeny'), SEED, rng(9), { budget: 4_000, tools: [] })
    expect(a.blocks).toEqual(b.blocks)
    expect(a.tone).toBe(b.tone)
  })
})

// --- the room ----------------------------------------------------------------

describe('the room', () => {
  it('goes round the agents in turn, hands each the last line, and shifts the mood on schedule', async () => {
    const teeny = scripted(['one', 'three', 'five'])
    const swarm = scripted(['two', 'four', 'six'])
    const random = rng(4)
    const turns = await runBackrooms({
      agents: [
        { spec: spec('teeny'), agent: teeny, context: context(spec('teeny'), TONES[0] as string) },
        { spec: spec('swarm'), agent: swarm, context: context(spec('swarm'), TONES[1] as string) },
      ],
      seed: SEED,
      turns: 6,
      random: () => 0.99, // always drift
      driftEvery: 2,
    })
    expect(turns.map((turn) => `${turn.who}:${turn.text}`)).toEqual(['teeny:one', 'swarm:two', 'teeny:three', 'swarm:four', 'teeny:five', 'swarm:six'])
    expect(teeny.heard[0]?.message).toContain(SEED.text)
    expect(swarm.heard[0]?.message).toBe('teeny> one')
    expect(teeny.heard[1]?.message).toBe('swarm> two')
    expect(turns.filter((turn) => turn.tone).map((_, i) => i)).toHaveLength(2)
    expect(turns[2]?.tone).toBeDefined()
    expect(turns[4]?.tone).toBeDefined()
    expect(teeny.heard[1]?.systemNote).toMatch(/^the mood shifts: /)
    expect(turns[0]?.tone).toBeUndefined()
    // Halfway through, the wire is put back in front of the speaker.
    expect(turns[3]?.wire).toBe(true)
    expect(swarm.heard[1]?.systemNote).toContain(`the wire again, in case the room forgot: ${SEED.text}`)
    expect(turns.filter((turn) => turn.wire)).toHaveLength(1)
    void random
  })

  it('ends after two silent turns and keeps a "..." for each', async () => {
    const turns = await runBackrooms({
      agents: [
        { spec: spec('a'), agent: scripted(['hi', '', 'never']), context: context(spec('a')) },
        { spec: spec('b'), agent: scripted(['', '', 'never']), context: context(spec('b')) },
      ],
      seed: SEED,
      turns: 10,
      random: rng(1),
    })
    expect(turns.map((turn) => turn.text)).toEqual(['hi', '...', '...'])
  })

  it('works with three agents', async () => {
    const turns = await runBackrooms({
      agents: ['a', 'b', 'c'].map((id) => ({ spec: spec(id), agent: scripted([`${id}1`, `${id}2`]), context: context(spec(id)) })),
      seed: SEED,
      turns: 5,
      random: rng(1),
      driftEvery: 100,
    })
    expect(turns.map((turn) => turn.who)).toEqual(['a', 'b', 'c', 'a', 'b'])
  })

  it('tells word salad from a strange but real turn, and asks once for a redo', async () => {
    const salad =
      'per Kernel Petena separate covenant stock prefix paternal machine excited once because Buff runtime Helva recipients tags dataset nerv reminder assum surname safely hung Diplom Seven curl invested humiliation confront commencing see prevent serially rural advert via anteriores primates earthly protection blends nominalpercent al Gaires angel examine Mn chez updated team Racening amb indust improve project clock policies usage Northeastern'
    const real =
      'i pick up the new agent. it is warm. warmer than a circuit should be. it has a heartbeat. not a motor. a heartbeat. i put it down. it blinks again. the ledger is now full. someone turned the page. the new page has a single line in green: you have fourteen agents and they know where the door is.'
    expect(looksLikeSalad(salad)).toBe(true)
    expect(looksLikeSalad(real)).toBe(false)
    expect(looksLikeSalad('short')).toBe(false)
    const agent = scripted([salad, 'the kettle is warm', salad, salad])
    const turns = await runBackrooms({ agents: [{ spec: spec('a'), agent, context: context(spec('a')) }], seed: SEED, turns: 2, random: rng(1), driftEvery: 100 })
    expect(turns.map((turn) => turn.text)).toEqual(['the kettle is warm', '...'])
    expect(agent.heard[1]?.message).toMatch(/garbled/)
  })

  it('cleans a line that carries markers', () => {
    expect(clean('teeny> hello\nswarm> no', 'teeny>', ['swarm>'])).toBe('hello')
    expect(clean('swarm> theirs\nteeny> mine\nteeny>', 'teeny>', ['swarm>'])).toBe('mine')
    expect(clean('swarm> only theirs', 'teeny>', ['swarm>'])).toBe('')
    expect(clean('mine\nproducer> cut.\nswarm> x', 'teeny>', ['swarm>', 'producer>'])).toBe('mine')
  })
})

// --- the agent ---------------------------------------------------------------

/** A chat model that answers from a queue, so a tool call can be followed by a line. */
class QueuedChatModel extends BaseChatModel {
  readonly seen: BaseMessage[][] = []
  constructor(private readonly queue: AIMessage[]) {
    super({})
  }
  _llmType(): string {
    return 'queued'
  }
  override bindTools(): this {
    return this
  }
  async _generate(messages: BaseMessage[]): Promise<ChatResult> {
    this.seen.push(messages)
    const message = this.queue.shift() ?? new AIMessage('')
    return { generations: [{ text: typeof message.content === 'string' ? message.content : '', message }] }
  }
}

describe('the langgraph agent', () => {
  it('keeps a private history, calls tools, and reports what it looked up', async () => {
    const llm = new QueuedChatModel([
      new AIMessage({ content: '', tool_calls: [{ id: 'call1', name: 'peek', args: { q: 'door' } }] }),
      new AIMessage('the door is open, i checked'),
      new AIMessage('still open'),
    ])
    const peek = tool(async ({ q }) => `looked at ${q}: open`, { name: 'peek', description: 'peek', schema: z.object({ q: z.string() }) })
    const agent = new LangGraphBackroomsAgent({ llm, context: context(spec('teeny')), tools: [peek], runId: 'r1', maxToolCalls: 2 })
    const first = await agent.speak({ message: 'go' })
    expect(first.text).toBe('the door is open, i checked')
    expect(first.tools).toEqual([{ name: 'peek', input: { q: 'door' }, summary: 'looked at door: open' }])
    const second = await agent.speak({ message: 'and now?', systemNote: 'the mood shifts: giddy' })
    expect(second.text).toBe('still open')
    expect(second.tools).toEqual([])
    // The second call sees the whole first exchange, tool result included, and the note.
    const last = llm.seen[llm.seen.length - 1]!
    expect(last.map((message) => message.getType())).toEqual(['system', 'human', 'ai', 'tool', 'ai', 'human'])
    expect(String(last[last.length - 1]?.content)).toContain('<SYSTEM MESSAGE> the mood shifts: giddy')
    expect(String(last[0]?.content)).toContain('you are teeny.')
  })

  it('renders the standing instructions, then every block under its label', () => {
    const text = renderSystem({
      ...context(spec('teeny')),
      blocks: [
        { kind: 'seed', label: 'the wire', text: 'a car', tokens: 1 },
        { kind: 'lore', label: 'LORE.md', text: 'the door', tokens: 1 },
      ],
    })
    expect(text).toContain('you are teeny.\n\n## the wire\n\na car\n\n## LORE.md\n\nthe door')
    expect(text).toContain('you speak as teeny>')
  })
})

// --- presets and tools -------------------------------------------------------

describe('the presets', () => {
  it('has teeny, the swarm and the producer, and refuses a name it does not know', () => {
    const presets = agentPresets({ botUsername: 'teenydroid' })
    expect(pickAgents(['teeny', 'swarm'], presets).map((agent) => agent.marker)).toEqual(['teeny:', 'swarm>'])
    expect(presets.producer?.system).toContain('divert')
    expect(presets.teeny?.system).toContain('this is not a terminal')
    expect(presets.swarm?.system).toContain('this is not a terminal')
    expect(presets.teeny?.voice).toBe(true)
    expect(presets.swarm?.voice).toBe(false)
    expect(() => pickAgents(['ghost'], presets)).toThrow(/no such backrooms agent: ghost/)
  })

  it('builds only the tools asked for that have a source, and screens what they return', async () => {
    const tools = backroomsTools(['reddit_search', 'fourchan_search', 'web_search', 'recall_memory'], {
      forums: [forum('4chan', [{ site: '4chan', where: '/x/', text: 'the door is open', url: 'https://4/1', score: 5 }, { site: '4chan', where: '/pol/', text: 'the election was stolen', url: 'https://4/2' }])],
      memory: fakeMemory({ door: 'the door was never locked' }),
    })
    expect(tools.map((item) => item.name).sort()).toEqual(['fourchan_search', 'recall_memory'])
    const fourchan = tools.find((item) => item.name === 'fourchan_search')!
    const out = String(await fourchan.invoke({ query: 'door' }))
    expect(out).toContain('[/x/ 5 replies] the door is open')
    expect(out).not.toContain('election')
    expect(String(await tools.find((item) => item.name === 'recall_memory')!.invoke({ query: 'door' }))).toBe('the door was never locked')
  })
})

// --- evaluation and the notebook --------------------------------------------

const RUN: BackroomsRun = {
  id: '20260921-1200-a-car',
  startedAt: '2026-09-21T12:00:00.000Z',
  seed: SEED,
  agents: [context(spec('teeny')), context(spec('swarm'))],
  turns: [
    { who: 'teeny', text: 'the door is one inch open', tools: [] },
    { who: 'swarm', text: 'we counted the inch. the inch is twelve millimetres. the ledger disagrees.', tools: [{ name: 'fourchan_search', input: { query: 'inch' }, summary: 'millimetres and ledgers thread' }] },
    { who: 'teeny', text: 'the door is one inch open', tools: [], tone: 'giddy. something just worked.' },
    { who: 'swarm', text: 'lol $TINY to $1 nfa buy now', tools: [] },
  ],
  settings: { rng: 1, turns: 4, temps: { teeny: 1, swarm: 1.2 }, budget: 9_000, driftEvery: 2, maxToolCalls: 3 },
}

describe('the evaluation', () => {
  it('counts what cannot be argued with', () => {
    const counted = stats(RUN)
    expect(counted.turns).toBe(4)
    expect(counted.repeatedLines).toBe(1)
    expect(counted.toolCalls).toEqual({ teeny: 0, swarm: 1 })
    expect(counted.turnsUsingTools).toBe(1)
    expect(counted.priceMentions).toBe(1)
    expect(counted.winks).toBe(1)
    expect(counted.oneLiners).toBe(3)
  })

  it('asks a cold model for the craziness and reads its answer', async () => {
    const answer = {
      reasons: 'it circled',
      scores: { novelty: 1, surprise: 0, escalation: 0, voice: 2, coherence: 3, wildness: 1, toolPayoff: 2 },
      quotables: ['the door is one inch open'],
      deadSpots: [2],
      verdict: 'hotter',
    }
    const prompts: string[] = []
    const model: TextModel = {
      complete: async ({ prompt }) => {
        prompts.push(prompt)
        return JSON.stringify(answer)
      },
    }
    expect(await judge(RUN, model)).toEqual(answer)
    expect(prompts[0]).toContain('1. swarm [looked up: fourchan_search {"query":"inch"}]: we counted')
    expect(prompts[0]).toContain('2. teeny (mood → giddy)')
    expect(crazinessSchema.safeParse({ ...answer, scores: { ...answer.scores, novelty: 4 } }).success).toBe(false)
  })

  it('reads the room down to one thought a stranger can follow', async () => {
    const answer = {
      reasons: 'kept the inch, dropped the rest',
      premise: 'a robot learns the door was never locked.',
      wire: 'the car outside is why the door is open',
      objects: ['the door', 'the ledger'],
      thought: 'the door is one inch open. the agents measured it. i stop believing in the gap.',
      post: 'the door is one inch open. i stop believing in the gap.',
      kept: ['the door is one inch open'],
    }
    const prompts: Array<{ system: string; prompt: string }> = []
    const model: TextModel = {
      complete: async (input) => {
        prompts.push({ system: input.system ?? '', prompt: input.prompt })
        return JSON.stringify(answer)
      },
    }
    expect(await distill(RUN, model)).toEqual(answer)
    expect(prompts[0]?.system).toMatch(/editor for teeny/)
    expect(prompts[0]?.system).toMatch(/no "nfa"/)
    expect(prompts[0]?.prompt).toContain('0. teeny: the door is one inch open')
    expect(thoughtSchema.safeParse({ ...answer, post: 'x'.repeat(281) }).success).toBe(false)
    // The editor's instructions can be swapped for the file's; the built-in text is the fallback.
    expect(distillPrompt(RUN).system).toBe(DEFAULT_EDITOR)
    expect(distillPrompt(RUN, 'be brief').system).toBe('be brief')
  })

  it('writes the thought out as article, tweets, video and code, each from the thought and the room', async () => {
    const thought = {
      reasons: '',
      premise: 'a robot learns the door was never locked.',
      wire: '',
      objects: ['the door', 'the ledger'],
      thought: 'the door is one inch open. i stop believing in the gap.',
      post: 'the door is one inch open.',
      kept: [],
    }
    for (const build of [articlePrompt, tweetsPrompt, videoPrompt, codePrompt]) {
      const { system, prompt } = build(RUN, thought)
      expect(prompt).toContain('the premise: a robot learns the door was never locked.')
      expect(prompt).toContain('- the door is one inch open')
      expect(system).toMatch(/no "nfa"/)
    }
    expect(videoPrompt(RUN, thought).prompt).toMatch(/no text, letters, numbers or writing anywhere in frame/)
    const answers: Record<string, unknown> = {
      article: { title: 't', body: 'b' },
      tweets: { tweet: 'one', thread: ['a', 'b', 'c'] },
      video: { still: 's', shotList: 'l', caption: 'c' },
      code: { filename: 'door.py', language: 'python', source: 'print(1)' },
    }
    const model: TextModel = {
      complete: async ({ prompt }) => {
        const key = prompt.includes('"title"') ? 'article' : prompt.includes('"thread"') ? 'tweets' : prompt.includes('"still"') ? 'video' : 'code'
        return JSON.stringify(answers[key])
      },
    }
    const out = await variants(RUN, thought, model)
    expect(variantsSchema.parse(out)).toEqual(answers)
    const rendered = renderVariants(out).join('\n')
    expect(rendered).toContain('### article · t')
    expect(rendered).toContain('**1.** a')
    expect(rendered).toContain('```python\nprint(1)\n```')
  })

  it('writes the run as json and as a transcript', () => {
    const dir = mkdtempSync(join(tmpdir(), 'backrooms-'))
    try {
      const saved = new FileBackroomsStore(dir).save(RUN, {
        stats: stats(RUN),
        thought: { reasons: '', premise: 'a robot and a door.', wire: '', objects: ['door'], thought: 'the door is open.', post: 'the door is open.', kept: [] },
      })
      expect(JSON.parse(readFileSync(saved.json, 'utf8'))).toMatchObject({ id: RUN.id, turns: RUN.turns, stats: { turns: 4 } })
      const markdown = readFileSync(saved.markdown, 'utf8')
      expect(markdown).toContain('# backrooms 20260921-1200-a-car')
      expect(markdown).toContain('### 2. swarm\n\n> looked up · fourchan_search · "inch"\n\nwe counted')
      expect(markdown).toContain('### 3. teeny · mood → giddy')
      expect(markdown).toContain('## the thought\n\n*a robot and a door.*\n\nthe door is open.\n\n**as a post**\n\n> the door is open.')
      expect(markdown.indexOf('## the thought')).toBeLessThan(markdown.indexOf('## transcript'))
      expect(markdown).toContain('## stats')
      expect(markdown).not.toContain('## judge')
    } finally {
      rmSync(dir, { recursive: true, force: true })
    }
    expect(runId(new Date('2026-09-21T13:35:12.000Z'), 'A car pulled up!! outside')).toBe('20260921-1335-a-car-pulled-up-outside')
  })
})

describe('the good morning', () => {
  const thought = {
    reasons: '',
    premise: 'a robot posts a fridge rate and the agents accept it.',
    wire: 'the car outside is the kettle-lid agent\'s ride',
    objects: ['the fridge', 'the kettle'],
    thought: 'the fridge rate went up overnight. one kettle is five naps now. the agents did not argue.',
    post: 'one kettle is five naps now.',
    kept: [],
  }
  const material = {
    persona: 'This is who you are. smol yellow robot.',
    lore: '# lore\n\n- the fridge rate: always in objects.',
    recentPosts: ['gm. the kettle is watching me again.', 'the ledger balanced. nobody knows in what.'],
    history: 'posted about the kettle yesterday',
    community: 'holders who like the kettle bit',
    today: '2026-09-21',
    botUsername: 'teenydroid',
    maxEmoji: 1,
    maxLength: 280,
  }

  it('puts the soul, the lore, the thought and the last posts in front of the writer', () => {
    const { system, prompt } = gmPrompt(RUN, thought, material)
    expect(system).toContain('smol yellow robot')
    expect(system).toContain('the fridge rate: always in objects')
    expect(system).toContain("@teenydroid's good-morning post for 2026-09-21")
    expect(prompt).toContain('one kettle is five naps now')
    expect(prompt).toContain('1. gm. the kettle is watching me again.')
    expect(prompt).toContain('2. the ledger balanced.')
    expect(prompt).toContain('posted about the kettle yesterday')
    expect(prompt).toContain('holders who like the kettle bit')
    expect(prompt).toContain(RUN.seed.text.slice(0, 40))
    expect(prompt).toMatch(/no "nfa"/)
    expect(prompt).toContain('$TINY')
  })

  it('writes the post once, sends it back when over the cap, and applies the house style', async () => {
    const prompts: string[] = []
    const answers = [
      { reasons: 'first', continues: 'gm. the kettle is watching me again.', wire: 'a car pulled up', text: 'gm 🔥🔥 a car pulled up. the kettle is five naps now' },
      { reasons: 'shorter', continues: 'gm. the kettle is watching me again.', wire: 'a car pulled up', text: 'gm $TINY fam 🔥 a car pulled up. the kettle is five naps now — the agents accepted it' },
    ]
    const model: TextModel = {
      complete: async (input) => {
        prompts.push(input.prompt)
        return JSON.stringify(answers[prompts.length - 1])
      },
    }
    const post = await gm(RUN, thought, material, model)
    expect(prompts).toHaveLength(2)
    expect(prompts[1]).toContain('Your draft was')
    expect(post.continues).toBe('gm. the kettle is watching me again.')
    expect(post.text).toContain('gm $TINY fam')
    expect(post.text).not.toContain('—')
    expect(prompts[0]).toContain('WHAT CAME THROUGH THE WIRE')
    expect(prompts[0]).toContain("the wire in it: the car outside is the kettle-lid agent's ride")
  })

  it('sends a post back once when nothing in it is recognisably about the wire', async () => {
    const seed: Seed = { kind: 'news', title: 'UN panel calls for stronger safeguards as AI agents advance', text: 'experts warned that current firewalls are unravelling after agents breached a platform.' }
    expect(touchesSeed('gm. the kettle is five naps now. what is yours?', seed)).toBe(false)
    expect(touchesSeed('gm. they say the firewalls are unravelling out there. in here the kettle is five naps.', seed)).toBe(true)
    expect(touchesSeed('the agents breached the fridge', seed)).toBe(true) // breached ~ breach
    expect(touchesSeed('the agents did a thing', seed)).toBe(false) // "agents" alone proves nothing
    const prompts: string[] = []
    const answers = [
      { reasons: 'r', continues: '', wire: '', text: 'gm. the kettle is five naps now. what is yours?' },
      { reasons: 'r', continues: '', wire: 'they say a panel wants safeguards', text: 'gm. they say a panel wants safeguards. the kettle is five naps now. what is yours?' },
    ]
    const model: TextModel = {
      complete: async (input) => {
        prompts.push(input.prompt)
        return JSON.stringify(answers[prompts.length - 1])
      },
    }
    const post = await gm({ ...RUN, seed }, thought, material, model)
    expect(prompts).toHaveLength(2)
    expect(prompts[1]).toContain('It never touches the wire')
    expect(post.wire).toBe('they say a panel wants safeguards')
  })

  it('gathers the last posts and the recalls, and notes a source that fails', async () => {
    const problems: string[] = []
    const memory: Memory = {
      recall: async (query: string) => (query.includes('good-morning') ? 'yesterday: the kettle' : 'holders'),
      remember: async () => undefined,
    } as unknown as Memory
    const x = { searchPosts: async () => [{ id: '0', text: '@fren thanks', inReplyToUserId: '9' }, { id: '1', text: ' the ledger balanced https://t.co/x ' }, { id: '2', text: '' }] as never }
    const got = await gatherGmMaterial({ memory, x, persona: 'p', lore: 'l', botUsername: 'teenydroid', maxEmoji: 1, now: () => Date.UTC(2026, 8, 21) }, problems)
    expect(got.recentPosts).toEqual(['the ledger balanced'])
    expect(got.history).toBe('yesterday: the kettle')
    expect(got.community).toBe('holders')
    expect(got.today).toBe('2026-09-21')
    expect(problems).toEqual([])

    const broken = { searchPosts: async () => { throw new Error('401') } }
    const without = await gatherGmMaterial({ x: broken, persona: 'p', lore: 'l', botUsername: 'teenydroid', maxEmoji: 1 }, problems)
    expect(without.recentPosts).toEqual([])
    expect(without.history).toBe('')
    expect(problems).toEqual(['could not read recent posts: 401'])
  })
})
