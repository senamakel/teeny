import { describe, expect, it, vi } from 'vitest'
import type { ArticleResult } from '../src/article/pipeline.js'
import type { Cadence } from '../src/cadence/cadence.js'
import { ArticleJob } from '../src/jobs/article-job.js'
import { STRUCTURE, THOUGHT } from './fixtures/article.js'

const done: ArticleResult = {
  outcome: 'posted',
  slug: '20260923-the-inch',
  engine: 'the box: teeny is being polite about leaving.',
  seed: { kind: 'news', title: 'a car pulled up', text: 'a car', source: 'the wire' },
  rooms: [{ index: 0, runId: 'r1', total: 20, saved: 'data/backrooms/r1.json' }],
  thought: THOUGHT,
  structure: { ...STRUCTURE, sections: STRUCTURE.sections.map((section, index) => ({ ...section, key: `s${index + 1}` })) },
  structureCheck: { ok: true, problems: [], fixes: [] },
  sections: ['a', 'b', 'c'],
  markdown: '## the gap\n\na',
  wordCount: 3,
  images: [],
  postId: '9001',
  xArticleId: 'a1',
  problems: [],
}

const logger = () => ({ info: vi.fn(), error: vi.fn() })

describe('the article job', () => {
  it('refuses to run twice inside the interval, in memory and from the store, and claims the cadence', async () => {
    const run = vi.fn().mockResolvedValue(done)
    const claim = vi.fn().mockResolvedValue({ ok: true })
    const within = vi.fn().mockImplementation(async (_kind: string, work: () => Promise<ArticleResult>) => work())
    const cadence = { claim, within } as unknown as Pick<Cadence, 'claim' | 'within'>
    let now = Date.parse('2026-09-23T23:00:00Z')
    const articles = { latestPublishedAt: vi.fn().mockResolvedValue(undefined) }
    const job = new ArticleJob({ pipeline: { run }, articles, logger: logger(), now: () => now, cadence })
    expect(await job.run()).toBe(done)
    now += 60_000
    expect(await job.run()).toBeUndefined()
    expect(run).toHaveBeenCalledTimes(1)
    expect(claim).toHaveBeenCalledWith('article')
    expect(within).toHaveBeenCalledWith('article', expect.any(Function))

    // A fresh process, but the store says one went out an hour ago.
    articles.latestPublishedAt.mockResolvedValue('2026-09-23T22:00:00.000Z')
    const restarted = new ArticleJob({ pipeline: { run }, articles, logger: logger(), now: () => now, cadence })
    expect(await restarted.run()).toBeUndefined()
    expect(run).toHaveBeenCalledTimes(1)
    expect(await restarted.run({ force: true })).toBe(done)
  })

  it('skips when the cadence refuses', async () => {
    const run = vi.fn().mockResolvedValue(done)
    const cadence = { claim: vi.fn().mockResolvedValue({ ok: false, reason: 'gap' }), within: vi.fn() } as unknown as Pick<Cadence, 'claim' | 'within'>
    const job = new ArticleJob({ pipeline: { run }, articles: { latestPublishedAt: async () => undefined }, logger: logger(), cadence })
    expect(await job.run()).toBeUndefined()
    expect(run).not.toHaveBeenCalled()
  })

  it('journals the run with the plan, the room and the title, and a failure when it throws', async () => {
    const record = vi.fn().mockResolvedValue(undefined)
    const readRun = vi.fn().mockResolvedValue({ turns: [{ who: 'teeny', text: 'the door is one inch open', tools: [] }] })
    const job = new ArticleJob({
      pipeline: { run: vi.fn().mockResolvedValue(done) },
      articles: { latestPublishedAt: async () => undefined },
      logger: logger(),
      journal: { record } as never,
      readRun,
    })
    await job.run()
    expect(readRun).toHaveBeenCalledWith('data/backrooms/r1.json')
    const entry = record.mock.calls[0]![0] as { trigger: string; said: string; outcome: string; thinking: string; tools: unknown[]; incoming: string }
    expect(entry).toMatchObject({ trigger: 'article', said: 'the inch\na door, a ruler, and a ledger that balances in something nobody can name.', outcome: 'posted' })
    expect(entry.thinking).toContain('title: the inch')
    expect(entry.thinking).toContain('the door is one inch open')
    expect(entry.tools).toEqual([{ name: 'article_publish', result: '9001' }])
    expect(entry.incoming).toContain('about the box')

    const rehearsal = new ArticleJob({
      pipeline: { run: vi.fn().mockResolvedValue({ ...done, outcome: 'dry-run', postId: undefined }) },
      articles: { latestPublishedAt: async () => undefined },
      logger: logger(),
      journal: { record } as never,
    })
    await rehearsal.run()
    expect(record.mock.calls[1]![0]).toMatchObject({ outcome: 'dry-run', outcomeNote: 'rehearsal saved as 20260923-the-inch' })

    const failing = new ArticleJob({
      pipeline: { run: vi.fn().mockRejectedValue(new Error('X refused the article (403): not enrolled')) },
      articles: { latestPublishedAt: async () => undefined },
      logger: logger(),
      journal: { record } as never,
    })
    await expect(failing.run()).rejects.toThrow(/403/)
    expect(record.mock.calls[2]![0]).toMatchObject({ trigger: 'article', outcome: 'failed', outcomeNote: 'X refused the article (403): not enrolled' })
  })
})
