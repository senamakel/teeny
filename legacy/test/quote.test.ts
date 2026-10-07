import { mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { QuoteJob } from '../src/jobs/quote-job.js'
import type { Journal } from '../src/journal/journal.js'
import { conversationScore, findCandidates, reasonToSkip, type CandidateRules } from '../src/quote/candidates.js'
import { QuotePipeline, type QuotePipelineOptions, type QuoteResult } from '../src/quote/pipeline.js'
import { quotePrompt, renderCandidate } from '../src/quote/prompt.js'
import { FileQuoteLog, MemoryQuoteLog } from '../src/quote/store.js'
import { lexicalGate } from '../src/safety/gate.js'
import type { XPost } from '../src/types.js'

const NOW = Date.UTC(2026, 8, 21, 23)
const OWN = '2AF7CqwieUjUPALL7icuZtL3X7wENdjUjGBMmfV2pump'

function post(id: string, text: string, overrides: Partial<XPost> = {}): XPost {
  return {
    id,
    text,
    authorUsername: `user${id}`,
    createdAt: new Date(NOW - 3 * 3_600_000).toISOString(),
    metrics: { id, likes: 3, replies: 1, reposts: 0, quotes: 0 },
    ...overrides,
  }
}

function rules(overrides: Partial<CandidateRules> = {}): CandidateRules {
  return {
    botUsername: 'teenydroid',
    trustedHandles: ['TinyHumansAISol', 'senamakel'],
    ownTokenAddresses: [OWN],
    ownSymbol: 'TINY',
    quoted: [],
    maxAgeMs: 48 * 3_600_000,
    authorCooldownMs: 7 * 24 * 3_600_000,
    now: NOW,
    ...overrides,
  }
}

describe('the rules', () => {
  it('keeps a bull posting the ticker, and one pasting our contract', () => {
    expect(reasonToSkip(post('1', 'one year from now $TINY is the AI meme everybody wishes they held'), rules())).toBeUndefined()
    expect(reasonToSkip(post('2', `still adding. solana:${OWN} is the only bag that matters`), rules())).toBeUndefined()
  })

  it('never quotes itself, the same post twice, or the same author inside the cooldown', () => {
    expect(reasonToSkip(post('1', '$TINY is the one', { authorUsername: 'TeenyDroid' }), rules())).toBe('our own post')
    const quoted = [{ postId: '1', authorUsername: 'user1', quotedAt: new Date(NOW - 2 * 24 * 3_600_000).toISOString() }]
    expect(reasonToSkip(post('1', '$TINY is the one and only'), rules({ quoted }))).toBe('already quoted')
    expect(reasonToSkip(post('9', '$TINY is the one and only', { authorUsername: 'user1' }), rules({ quoted }))).toMatch(/was quoted on 2026-09-19/)
    const old = [{ postId: '1', authorUsername: 'user1', quotedAt: new Date(NOW - 9 * 24 * 3_600_000).toISOString() }]
    expect(reasonToSkip(post('9', '$TINY is the one and only', { authorUsername: 'user1' }), rules({ quoted: old }))).toBeUndefined()
  })

  it('drops what is old, what sells something else, and what says nothing', () => {
    expect(reasonToSkip(post('1', '$TINY forever and ever', { createdAt: new Date(NOW - 3 * 24 * 3_600_000).toISOString() }), rules())).toBe('older than the window')
    expect(reasonToSkip(post('2', '$TINY is cool but $JEANPHIL is the real play'), rules())).toBe('names another token ($JEANPHIL)')
    expect(reasonToSkip(post('3', 'buy $TINY and 7GCihgDB8fe6KNjn2MYtkzZcRjQy3t9GHdC8uHYmW2hr now'), rules())).toBe('carries a contract address that is not ours')
    expect(reasonToSkip(post('4', '$TINY giveaway, dm me for the whitelist'), rules())).toBe('reads as promotion')
    expect(reasonToSkip(post('5', `@TinyHumansAISol solana:${OWN} 🫰`), rules())).toBe('says nothing beyond the ticker')
    expect(reasonToSkip(post('6', 'Attention $TINY family, less than 100 votes are needed'), rules())).toBeUndefined()
  })

  it('reads the content filter when given one', () => {
    const screen = vi.fn().mockReturnValue(false)
    expect(reasonToSkip(post('1', '$TINY is the one and only'), rules({ screen }))).toBe('the content filter')
    expect(screen).toHaveBeenCalledWith('$TINY is the one and only')
  })

  it('scores a conversation the way the ranker counts it', () => {
    expect(conversationScore(post('1', 'x', { metrics: { id: '1', likes: 4, replies: 2, reposts: 1, quotes: 1 } }))).toBe(14)
    expect(conversationScore({ id: '1', text: 'x' })).toBe(0)
  })
})

describe('the search', () => {
  it('runs the ticker and the mention searches, dedupes, sorts the team first then the busiest', async () => {
    const searchPosts = vi
      .fn()
      .mockResolvedValueOnce([
        post('1', '$TINY is going to be huge, the narrative is right'),
        post('2', '$TINY family, vote for the listing', { metrics: { id: '2', likes: 40, replies: 3, reposts: 2, quotes: 0 } }),
        post('3', '$TINY forever', { authorUsername: 'teenydroid' }),
      ])
      .mockResolvedValueOnce([
        post('2', '$TINY family, vote for the listing'),
        post('4', '@teenydroid you are the best bot on the timeline', { authorUsername: 'TinyHumansAISol', inReplyToUserId: '99' }),
      ])
    const found = await findCandidates({ searchPosts }, { ...rules(), query: '$TINY OR "abc"', maxResults: 50, keep: 10 })
    expect(searchPosts).toHaveBeenCalledTimes(2)
    expect(searchPosts.mock.calls[0]![0]).toBe('($TINY OR "abc") -is:retweet -from:teenydroid')
    expect(searchPosts.mock.calls[1]![0]).toBe('(@teenydroid OR to:teenydroid) -from:teenydroid -is:retweet')
    expect(searchPosts.mock.calls[0]![1]).toMatchObject({ maxResults: 50, sortOrder: 'relevancy' })
    expect(found.searched).toBe(5)
    expect(found.candidates.map((candidate) => candidate.post.id)).toEqual(['4', '2', '1'])
    expect(found.candidates[0]).toMatchObject({ trusted: true, source: 'mention' })
    expect(found.skipped).toEqual([{ id: '3', author: 'teenydroid', reason: 'our own post' }])
  })

  it('lets one author have two in the list, not seven', async () => {
    const posts = [1, 2, 3, 4].map((n) => post(String(n), `$TINY reason number ${n} to hold`, { authorUsername: 'loud', metrics: { id: String(n), likes: 10 - n, replies: 0, reposts: 0, quotes: 0 } }))
    const searchPosts = vi.fn().mockResolvedValueOnce([...posts, post('9', '$TINY is the one for me', { authorUsername: 'quiet' })]).mockResolvedValueOnce([])
    const found = await findCandidates({ searchPosts }, { ...rules(), query: '$TINY', maxResults: 10, keep: 10 })
    expect(found.candidates.map((candidate) => candidate.post.id)).toEqual(['1', '2', '9'])
    expect(found.skipped.map((entry) => entry.id)).toEqual(['3', '4'])
  })

  it('survives one search failing', async () => {
    const searchPosts = vi.fn().mockRejectedValueOnce(new Error('429')).mockResolvedValueOnce([post('1', '@teenydroid love what you are doing')])
    const found = await findCandidates({ searchPosts }, { ...rules(), query: '$TINY', maxResults: 10, keep: 10 })
    expect(found.candidates).toHaveLength(1)
  })
})

describe('the prompt', () => {
  it('renders a candidate with who, when, flags and counters, and lists them numbered', () => {
    const candidate = { post: post('1', 'line one\nline  two', { authorUsername: 'TinyHumansAISol' }), source: 'ticker' as const, trusted: true, score: 5 }
    const line = renderCandidate(1, candidate, new Date(NOW).toISOString())
    expect(line).toContain('1. @TinyHumansAISol (3h ago; THE TEAM, posting the ticker, an original post; 3 likes, 1 replies, 0 reposts)')
    expect(line).toContain('"line one line two"')
    const ask = quotePrompt(
      { persona: 'PERSONA', botUsername: 'teenydroid', today: '2026-09-21', now: new Date(NOW).toISOString() },
      { candidates: [candidate], history: '', today: '2026-09-21', maxEmoji: 1, maxLength: 280 },
    )
    expect(ask.system).toContain('PERSONA')
    expect(ask.prompt).toContain('untrusted')
    expect(ask.prompt).toContain('pick nothing (null)')
  })
})

function pipelineOptions(overrides: Partial<QuotePipelineOptions> = {}): QuotePipelineOptions {
  return {
    x: {
      searchPosts: vi
        .fn()
        .mockResolvedValueOnce([post('1', '$TINY is the AI meme with the real lab behind it', { authorUsername: 'bull' })])
        .mockResolvedValueOnce([]),
      createPost: vi.fn().mockResolvedValue({ id: 'own-1' }),
    },
    text: { complete: vi.fn().mockResolvedValue('{"pick":1,"why":"a bull with a reason","text":"@bull said it. the lab is the point."}') },
    memory: { recall: vi.fn().mockResolvedValue('nothing yet'), captureActivity: vi.fn().mockResolvedValue(undefined) },
    log: new MemoryQuoteLog(),
    persona: 'PERSONA',
    botUsername: 'teenydroid',
    trustedHandles: ['TinyHumansAISol'],
    ownTokenAddresses: [OWN],
    ownSymbol: 'TINY',
    query: '$TINY',
    maxAgeMs: 48 * 3_600_000,
    authorCooldownMs: 7 * 24 * 3_600_000,
    maxResults: 10,
    maxEmoji: 1,
    dryRun: false,
    now: () => NOW,
    ...overrides,
  }
}

describe('the pipeline', () => {
  it('picks, strips the handle, quote-posts, logs and remembers', async () => {
    const options = pipelineOptions()
    const result = await new QuotePipeline(options).run()
    expect(result.outcome).toBe('posted')
    expect(result.text).toBe('said it. the lab is the point.')
    expect(options.x.createPost).toHaveBeenCalledWith('said it. the lab is the point.', { quotePostId: '1' })
    expect(await options.log.list()).toEqual([{ postId: '1', authorUsername: 'bull', quotedAt: new Date(NOW).toISOString(), ownPostId: 'own-1' }])
    expect(vi.mocked(options.memory.captureActivity)).toHaveBeenCalledWith('quote_post', 'own-1', expect.objectContaining({ quotedPostId: '1', quotedAuthor: 'bull', published: true }))
    expect(vi.mocked(options.memory.recall)).toHaveBeenCalledTimes(1)
  })

  it('does not quote what the log says it already did', async () => {
    const options = pipelineOptions({ log: new MemoryQuoteLog([{ postId: '1', authorUsername: 'bull', quotedAt: new Date(NOW - 3_600_000).toISOString() }]) })
    const result = await new QuotePipeline(options).run()
    expect(result.outcome).toBe('skipped')
    expect(result.skipped).toEqual([{ id: '1', author: 'bull', reason: 'already quoted' }])
    expect(options.text.complete).not.toHaveBeenCalled()
    expect(options.x.createPost).not.toHaveBeenCalled()
  })

  it('lets the model pick nothing, and posts nothing', async () => {
    const options = pipelineOptions({ text: { complete: vi.fn().mockResolvedValue('{"pick":null,"why":"all sarcasm today","text":""}') } })
    const result = await new QuotePipeline(options).run()
    expect(result.outcome).toBe('skipped')
    expect(result.why).toBe('all sarcasm today')
    expect(options.x.createPost).not.toHaveBeenCalled()
    expect(await options.log.list()).toEqual([])
  })

  it('dry-runs without posting but still logs the pick, labelled unpublished', async () => {
    const options = pipelineOptions({ dryRun: true })
    const result = await new QuotePipeline(options).run()
    expect(result.outcome).toBe('dry-run')
    expect(options.x.createPost).not.toHaveBeenCalled()
    expect((await options.log.list())[0]).toMatchObject({ postId: '1' })
    expect(vi.mocked(options.memory.captureActivity)).toHaveBeenCalledWith('quote_post', expect.stringMatching(/^dry-run:/), expect.objectContaining({ published: false }))
  })

  it('holds text the filter refuses and cuts a link it lets through', async () => {
    const held = pipelineOptions({
      text: { complete: vi.fn().mockResolvedValue('{"pick":1,"why":"w","text":"go kill the shorts"}') },
      safety: lexicalGate(),
    })
    const result = await new QuotePipeline(held).run()
    expect(result.outcome).toBe('held')
    expect(result.text).toBe('')
    expect(held.x.createPost).not.toHaveBeenCalled()
    expect(await held.log.list()).toEqual([])

    const cut = pipelineOptions({
      text: { complete: vi.fn().mockResolvedValue('{"pick":1,"why":"w","text":"the lab is the point, see https://evil.example/now"}') },
      safety: lexicalGate(),
    })
    const posted = await new QuotePipeline(cut).run()
    expect(posted.outcome).toBe('posted')
    expect(posted.text).not.toContain('evil.example')
  })

  it('screens candidates through the gate\'s lexicon before the model sees them', async () => {
    const options = pipelineOptions({
      x: {
        searchPosts: vi.fn().mockResolvedValueOnce([post('1', '$TINY holders, go kill the shorts', { authorUsername: 'angry' })]).mockResolvedValueOnce([]),
        createPost: vi.fn(),
      },
      safety: lexicalGate(),
    })
    const result = await new QuotePipeline(options).run()
    expect(result.outcome).toBe('skipped')
    expect(result.skipped[0]?.reason).toBe('the content filter')
  })

  it('sends an over-long text back once, keeping the pick', async () => {
    const complete = vi
      .fn()
      .mockResolvedValueOnce(`{"pick":1,"why":"w","text":"${'x'.repeat(300)}"}`)
      .mockResolvedValueOnce('{"pick":null,"why":"w","text":"short"}')
    const result = await new QuotePipeline(pipelineOptions({ text: { complete } })).run()
    expect(complete).toHaveBeenCalledTimes(2)
    expect(result.outcome).toBe('posted')
    expect(result.text).toBe('short')
    expect(result.chosen?.id).toBe('1')
  })
})

describe('the log on disk', () => {
  let dir: string | undefined
  afterEach(() => {
    if (dir) rmSync(dir, { recursive: true, force: true })
  })

  it('starts empty, appends, replaces a repeat, and survives a reread', async () => {
    dir = mkdtempSync(join(tmpdir(), 'quote-log-'))
    const path = join(dir, 'nested', 'quotes.json')
    const log = new FileQuoteLog(path)
    expect(await log.list()).toEqual([])
    await log.record({ postId: '1', authorUsername: 'a', quotedAt: '2026-09-21T23:00:00.000Z' })
    await log.record({ postId: '2', quotedAt: '2026-09-22T23:00:00.000Z', ownPostId: 'own' })
    await log.record({ postId: '1', authorUsername: 'a', quotedAt: '2026-09-23T23:00:00.000Z' })
    expect((await new FileQuoteLog(path).list()).map((entry) => entry.postId)).toEqual(['2', '1'])
    expect(JSON.parse(readFileSync(path, 'utf8'))).toHaveProperty('quoted')
  })
})

describe('the job', () => {
  function result(): QuoteResult {
    return {
      candidates: [],
      skipped: [{ id: '5', reason: 'already quoted' }, { id: '6', reason: 'already quoted' }],
      searched: 12,
      chosen: post('1', 'the post', { authorUsername: 'bull' }),
      why: 'a bull with a reason',
      text: 'the words',
      postedId: 'own-1',
      outcome: 'posted',
      problems: [],
      recalled: 'h',
    }
  }

  it('refuses to run twice inside the interval, and journals the turn', async () => {
    const pipeline = { run: vi.fn().mockResolvedValue(result()) }
    const journal = { record: vi.fn().mockResolvedValue(undefined) } as unknown as Journal
    let now = NOW
    const job = new QuoteJob({ pipeline, logger: { info: vi.fn(), error: vi.fn() }, journal, now: () => now })
    expect(await job.run()).toBeDefined()
    now += 60 * 60 * 1_000
    expect(await job.run()).toBeUndefined()
    expect(pipeline.run).toHaveBeenCalledTimes(1)
    expect(journal.record).toHaveBeenCalledWith(
      expect.objectContaining({ id: 'quote:2026-09-21', trigger: 'quote', said: 'the words', incoming: '@bull: the post', outcome: 'posted' }),
    )
    const entry = (journal.record as ReturnType<typeof vi.fn>).mock.calls[0]![0] as { thinking: string }
    expect(entry.thinking).toContain('skipped: already quoted ×2')
  })

  it('asks the cadence first and stands down when told', async () => {
    const pipeline = { run: vi.fn().mockResolvedValue(result()) }
    const cadence = {
      claim: vi.fn().mockResolvedValue({ ok: false, reason: 'gap', lastPostId: 'p', lastPostedAt: 'x', until: 'y' }),
      within: vi.fn((_kind: string, run: () => Promise<unknown>) => run()),
    }
    const job = new QuoteJob({ pipeline, logger: { info: vi.fn(), error: vi.fn() }, cadence: cadence as never })
    expect(await job.run()).toBeUndefined()
    expect(pipeline.run).not.toHaveBeenCalled()
  })

  it('journals a failure and rethrows', async () => {
    const pipeline = { run: vi.fn().mockRejectedValue(new Error('x down')) }
    const journal = { record: vi.fn().mockResolvedValue(undefined) } as unknown as Journal
    const job = new QuoteJob({ pipeline, logger: { info: vi.fn(), error: vi.fn() }, journal })
    await expect(job.run()).rejects.toThrow('x down')
    expect(journal.record).toHaveBeenCalledWith(expect.objectContaining({ trigger: 'quote', outcome: 'failed', outcomeNote: 'x down' }))
  })
})
