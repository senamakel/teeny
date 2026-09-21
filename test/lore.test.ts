import { describe, expect, it, vi } from 'vitest'
import { lore, lorePrompt } from '../src/backrooms/lore.js'
import type { BackroomsRun, Seed } from '../src/backrooms/types.js'
import { LoreJob } from '../src/jobs/lore-job.js'
import type { Journal } from '../src/journal/journal.js'
import { LorePipeline, type LorePipelineOptions, type LoreResult } from '../src/lore/pipeline.js'
import type { TextModel } from '../src/media/reference.js'
import { lexicalGate } from '../src/safety/gate.js'

const NOW = Date.UTC(2026, 8, 22, 15)
const SEED: Seed = { kind: 'news', title: 'AI agents now file taxes', text: 'A startup says its agents filed a return unsupervised.', source: 'the wire' }
const RUN: BackroomsRun = {
  id: '20260922-1500-taxes',
  startedAt: '2026-09-22T15:00:00.000Z',
  seed: SEED,
  agents: [],
  turns: [{ who: 'teeny', text: 'the agents filed the kettle', tools: [] }],
  settings: { rng: 1, turns: 1, temps: {}, budget: 9_000, driftEvery: 2, maxToolCalls: 0 },
}
const thought = {
  reasons: '',
  premise: 'the tiny agents file the kettle as an asset and the ledger balances.',
  wire: 'agents filing taxes unsupervised',
  objects: ['the kettle', 'the ledger'],
  thought: 'the agents put the kettle on the ledger tonight. in crayon. it balanced. nobody asked in what.',
  post: 'the kettle is on the ledger now.',
  kept: [],
}
const material = {
  persona: 'This is who you are. smol yellow robot.',
  lore: '# lore\n\n- the ledger: crayon, balances, nobody knows in what.',
  recentPosts: ['gm. the kettle is watching me again.'],
  history: 'posted about the crown yesterday',
  community: 'holders who like the kettle bit',
  today: '2026-09-22',
  botUsername: 'teenydroid',
  maxEmoji: 1,
  maxLength: 280,
}

describe('the lore writer', () => {
  it('puts the soul, the lore, the thought and the last posts in front of the writer, with no good morning asked for', () => {
    const { system, prompt } = lorePrompt(RUN, thought, material)
    expect(system).toContain('smol yellow robot')
    expect(system).toContain('crayon, balances')
    expect(system).toContain("@teenydroid's afternoon post for 2026-09-22")
    expect(prompt).toContain('the kettle on the ledger tonight')
    expect(prompt).toContain('1. gm. the kettle is watching me again.')
    expect(prompt).toContain('posted about the crown yesterday')
    expect(prompt).toContain('AI agents now file taxes')
    expect(prompt).toContain('Not on a question.')
    expect(prompt).not.toMatch(/good morning/i)
  })

  it('sends an over-long draft back, cuts a closing ask, and keeps the lab', async () => {
    const answers = [
      { reasons: 'r', continues: '', text: `the kettle is on the ledger. ${'x'.repeat(300)}` },
      { reasons: 'r', continues: '', text: 'the kettle is on the ledger now. in crayon.\nwhat is yours worth?' },
      { reasons: 'r', continues: '', text: 'the kettle is on the ledger now. in crayon.\nrate it?' },
    ]
    let calls = 0
    const model: TextModel = { complete: async () => JSON.stringify(answers[calls++]) }
    const post = await lore(RUN, thought, material, model)
    expect(calls).toBe(3)
    expect(post.text).toBe('the kettle is on the ledger now. in crayon.')
  })

  it('rewrites once when the draft lost the thought, and keeps the draft when the rewrite did too', async () => {
    const answers = [
      { reasons: 'r', continues: '', text: 'a startup says its agents filed a return. fine.' },
      { reasons: 'r', continues: '', text: 'the return was filed. fine.' },
    ]
    let calls = 0
    const model: TextModel = { complete: async () => JSON.stringify(answers[calls++]) }
    const post = await lore(RUN, thought, material, model)
    expect(calls).toBe(2)
    expect(post.text).toBe('a startup says its agents filed a return. fine.')
  })
})

function pipelineOptions(overrides: Partial<LorePipelineOptions> = {}): LorePipelineOptions {
  return {
    backrooms: {
      run: vi.fn().mockResolvedValue({ run: RUN, thought, problems: ['reddit was down'], stats: {}, saved: { json: 'r.json', markdown: 'r.md' } }),
      lore: vi.fn().mockResolvedValue({ reasons: 'kept the kettle', continues: '', text: 'the kettle is on the ledger now. in crayon.' }),
    },
    seeds: {
      news: vi.fn().mockResolvedValue(undefined),
      forum: vi.fn().mockResolvedValue(SEED),
    },
    sources: ['news', 'forum'],
    topics: ['ai agents'],
    x: { createPost: vi.fn().mockResolvedValue({ id: 'own-1' }) },
    memory: { captureActivity: vi.fn().mockResolvedValue(undefined) },
    turns: 6,
    dryRun: false,
    random: () => 0,
    now: () => NOW,
    ...overrides,
  }
}

describe('the pipeline', () => {
  it('tries the sources in order, opens the room on the first seed, writes the post and posts it', async () => {
    const options = pipelineOptions()
    const result = await new LorePipeline(options).run()
    expect(result.outcome).toBe('posted')
    expect(result.from).toEqual({ source: 'forum', topic: 'ai agents' })
    expect(options.seeds.news).toHaveBeenCalledWith('ai agents')
    expect(vi.mocked(options.backrooms.run)).toHaveBeenCalledWith(SEED, { turns: 6 })
    expect(options.x.createPost).toHaveBeenCalledWith('the kettle is on the ledger now. in crayon.')
    expect(result.problems).toEqual(['news had nothing on "ai agents"', 'the room: reddit was down'])
    expect(options.memory.captureActivity).toHaveBeenCalledWith('lore_post', 'own-1', expect.objectContaining({ premise: thought.premise, published: true }))
  })

  it('stays quiet when nothing comes through the wire, and when the room leaves no thought', async () => {
    const dry = pipelineOptions({ seeds: { news: vi.fn().mockResolvedValue(undefined), forum: vi.fn().mockRejectedValue(new Error('4chan 503')) } })
    const quiet = await new LorePipeline(dry).run()
    expect(quiet.outcome).toBe('skipped')
    expect(quiet.problems).toContain('forum could not be read: 4chan 503')
    expect(dry.backrooms.run).not.toHaveBeenCalled()

    const empty = pipelineOptions()
    vi.mocked(empty.backrooms.run).mockResolvedValue({ run: RUN, problems: [], stats: {}, saved: { json: 'r.json', markdown: 'r.md' } } as never)
    const none = await new LorePipeline(empty).run()
    expect(none.outcome).toBe('skipped')
    expect(none.problems).toContain('the room ran but left no thought')
    expect(empty.x.createPost).not.toHaveBeenCalled()
  })

  it('dry-runs without posting and still remembers, labelled unpublished', async () => {
    const options = pipelineOptions({ dryRun: true })
    const result = await new LorePipeline(options).run()
    expect(result.outcome).toBe('dry-run')
    expect(options.x.createPost).not.toHaveBeenCalled()
    expect(options.memory.captureActivity).toHaveBeenCalledWith('lore_post', expect.stringMatching(/^dry-run:/), expect.objectContaining({ published: false }))
  })

  it('holds a post the filter refuses', async () => {
    const options = pipelineOptions({ safety: lexicalGate() })
    vi.mocked(options.backrooms.lore).mockResolvedValue({ reasons: 'r', continues: '', text: 'the agents will kill the humans tonight' })
    const result = await new LorePipeline(options).run()
    expect(result.outcome).toBe('held')
    expect(result.text).toBe('')
    expect(options.x.createPost).not.toHaveBeenCalled()
  })
})

describe('the job', () => {
  function result(): LoreResult {
    return {
      seed: SEED,
      from: { source: 'news', topic: 'ai agents' },
      room: { id: RUN.id, thought, saved: 'r.md' },
      post: { reasons: 'kept the kettle', continues: 'the kettle is watching me again', text: 'the words' },
      text: 'the words',
      postedId: 'own-1',
      outcome: 'posted',
      problems: [],
    }
  }
  const rota = { days: 7, anchor: '2026-09-14' }
  const day = (date: string): number => Date.parse(`${date}T15:00:00Z`)

  it('runs on its slots only', async () => {
    const pipeline = { run: vi.fn().mockResolvedValue(result()) }
    let now = day('2026-09-21') // slot 0, the moon's
    const job = new LoreJob({ pipeline, logger: { info: vi.fn(), error: vi.fn() }, rota: { rota, slots: [1, 2, 4, 5, 6] }, now: () => now })
    expect(await job.run()).toBeUndefined()
    now = day('2026-09-22') // slot 1
    expect(await job.run()).toBeDefined()
    now = day('2026-09-24') // slot 3, the chart's
    expect(await job.run()).toBeUndefined()
    expect(pipeline.run).toHaveBeenCalledTimes(1)
  })

  it('refuses to run twice inside the interval, and journals the turn', async () => {
    const pipeline = { run: vi.fn().mockResolvedValue(result()) }
    const journal = { record: vi.fn().mockResolvedValue(undefined) } as unknown as Journal
    let now = NOW
    const job = new LoreJob({ pipeline, logger: { info: vi.fn(), error: vi.fn() }, journal, now: () => now })
    expect(await job.run()).toBeDefined()
    now += 60 * 60 * 1_000
    expect(await job.run()).toBeUndefined()
    expect(pipeline.run).toHaveBeenCalledTimes(1)
    expect(journal.record).toHaveBeenCalledWith(
      expect.objectContaining({ id: 'lore:2026-09-22', trigger: 'lore', said: 'the words', incoming: 'news (the wire): AI agents now file taxes', outcome: 'posted' }),
    )
    const entry = vi.mocked(journal.record).mock.calls[0]![0] as { thinking: string; tools: Array<{ result: string }> }
    expect(entry.thinking).toContain('kept the kettle')
    expect(entry.thinking).toContain('continues: the kettle is watching me again')
    expect(entry.tools[0]!.result).toContain(thought.premise)
  })

  it('journals a failure and rethrows', async () => {
    const pipeline = { run: vi.fn().mockRejectedValue(new Error('ladder down')) }
    const journal = { record: vi.fn().mockResolvedValue(undefined) } as unknown as Journal
    const job = new LoreJob({ pipeline, logger: { info: vi.fn(), error: vi.fn() }, journal })
    await expect(job.run()).rejects.toThrow('ladder down')
    expect(journal.record).toHaveBeenCalledWith(expect.objectContaining({ trigger: 'lore', outcome: 'failed', outcomeNote: 'ladder down' }))
  })
})
