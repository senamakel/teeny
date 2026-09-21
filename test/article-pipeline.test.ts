import { describe, expect, it, vi } from 'vitest'
import { ArticlePipeline, splitSections, type ArticlePipelineOptions } from '../src/article/pipeline.js'
import { MemoryArticleStore } from '../src/article/store.js'
import type { RunResult } from '../src/backrooms/wiring.js'
import type { Critique } from '../src/critic/algo.js'
import type { TextModel } from '../src/media/reference.js'
import { MediaBudget, MediaStore } from '../src/media/store.js'
import { parseLore } from '../src/narrative/voice.js'
import { RUN, STRUCTURE, THOUGHT } from './fixtures/article.js'

const PNG = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0])

const LORE = `# lore

tiny agents run without humans.

## goals

- mint my own currency

## engines

- the box: teeny is being polite about leaving.
- the kettle: it boils.

## cast

- teeny: smol.

## running bits

- the kettle is always cold.

## latent

- the-door | trigger: episode >= 1 | the door was never locked.
`

const JUDGE = { reasons: '', scores: { novelty: 3, surprise: 3, escalation: 3, voice: 3, coherence: 3, wildness: 3, toolPayoff: 3 }, quotables: [], deadSpots: [], verdict: 'keep' }

function room(id: string, total: number, thought = THOUGHT): RunResult {
  const scores = { ...JUDGE.scores, novelty: total - 18 }
  return { run: { ...RUN, id }, stats: {} as RunResult['stats'], judge: { ...JUDGE, scores }, thought, saved: { json: `data/backrooms/${id}.json`, markdown: `data/backrooms/${id}.md` } }
}

/** A text model that answers by which shape the prompt asks for. */
function staged(overrides: Partial<Record<'structure' | 'check' | 'section' | 'proof' | 'fix' | 'brief', (prompt: string, system: string) => string>> = {}): TextModel & { asked: string[] } {
  const asked: string[] = []
  return {
    asked,
    complete: vi.fn(async ({ system, prompt }: { system?: string; prompt: string }) => {
      const sys = system ?? ''
      const answer = (kind: keyof typeof overrides, fallback: () => string): string => {
        asked.push(kind)
        return (overrides[kind] ?? fallback)(prompt, sys)
      }
      if (prompt.includes('"angle"')) return answer('structure', () => JSON.stringify({ ...STRUCTURE, sections: STRUCTURE.sections.map((s) => ({ ...s, key: '' })) }))
      if (prompt.includes('"problems"')) return answer('check', () => JSON.stringify({ ok: true, problems: [], fixes: [] }))
      if (prompt.includes('"brief": string')) return answer('brief', () => JSON.stringify({ brief: 'a warm picture' }))
      if (prompt.includes('"findings"')) return answer('proof', () => JSON.stringify({ verdict: 'clean', findings: [], voice: 'yes' }))
      if (sys.includes('apply exactly the findings')) return answer('fix', () => prompt.split('the text:\n\n')[1] ?? '')
      if (prompt.includes("answer with the section's text") || prompt.includes("answer with the cut section's text")) {
        return answer('section', () => {
          const which = /write section (\d)/.exec(prompt)?.[1] ?? /section "([^"]+)"/.exec(prompt)?.[1] ?? '?'
          return `body of section ${which}. the kettle boiled and nobody asked it to.`
        })
      }
      throw new Error(`unexpected prompt: ${prompt.slice(0, 80)}`)
    }),
  }
}

interface Harness extends ArticlePipelineOptions {
  x: { uploadMedia: ReturnType<typeof vi.fn>; publishArticle: ReturnType<typeof vi.fn> }
  articles: MemoryArticleStore
  backrooms: { run: ReturnType<typeof vi.fn> }
  captured: ReturnType<typeof vi.fn>
}

function options(over: Partial<ArticlePipelineOptions> = {}): Harness {
  const model = staged()
  const captured = vi.fn(async () => undefined)
  const x = {
    uploadMedia: vi.fn(async () => ({ mediaId: `m${x.uploadMedia.mock.calls.length}` })),
    publishArticle: vi.fn(async () => ({ articleId: 'a1', postId: '9001' })),
  }
  const backrooms = { run: vi.fn(async () => (backrooms.run.mock.calls.length === 1 ? room('run-one', 20) : room('run-two', 21))) }
  const base: Harness = {
    captured,
    backrooms,
    seed: vi.fn(async () => ({ kind: 'news' as const, title: 'a car pulled up', text: 'a car pulled up outside a lab', source: 'the wire' })),
    topics: ['ai agents'],
    loreText: LORE,
    lore: parseLore(LORE),
    state: { load: async () => ({ season: 1, episode: 1, surfacedLore: ['the-door'], openThreads: [] }) },
    writer: model,
    editor: model,
    fixer: model,
    humanizer: '# Humanizer\n\n### 14. Em dashes',
    images: { generate: vi.fn(async () => ({ data: PNG, mimeType: 'image/png' as const })) },
    store: new MediaStore(),
    budget: new MediaBudget({ imagesPerDay: 20, videosPerDay: 0 }),
    x,
    articles: new MemoryArticleStore(),
    memory: { recall: vi.fn(), remember: vi.fn(), captureActivity: captured } as never,
    rooms: 2,
    sectionsMax: 4,
    words: 900,
    publish: false,
    random: () => 0.1,
    now: () => Date.parse('2026-09-23T23:00:00Z'),
  }
  return Object.assign(base, over) as Harness
}

describe('the article pipeline', () => {
  it('rehearses: rooms, plan, sections, proof, pictures, a dry-run record, nothing to x', async () => {
    const o = options()
    const result = await new ArticlePipeline(o).run()
    expect(result.outcome).toBe('dry-run')
    expect(o.backrooms.run).toHaveBeenCalledTimes(2)
    expect(o.backrooms.run.mock.calls[0]![1]).toMatchObject({ score: true, distill: true, variants: false, engine: expect.stringMatching(/^the (box|kettle)/) })
    expect(result.rooms.map((room) => room.total)).toEqual([20, 21])
    expect(result.thought).toEqual(THOUGHT)
    expect(result.structure?.sections.map((section) => section.key)).toEqual(['s1', 's2', 's3'])
    expect(result.structureCheck).toEqual({ ok: true, problems: [], fixes: [] })
    expect(result.sections).toHaveLength(3)
    expect(result.markdown).toContain('## the gap\n\n![the door](image:s1)\n\nbody of section 1.')
    expect(result.proof?.review.verdict).toBe('clean')
    // The cover and one per section.
    expect(o.images!.generate).toHaveBeenCalledTimes(4)
    expect(result.cover).toBeDefined()
    expect(result.images.map((image) => image.key)).toEqual(['s1', 's2', 's3'])
    expect(o.budget.remaining('image')).toBe(16)
    expect(result.contentState?.blocks.filter((block) => block.type === 'atomic')).toHaveLength(3)
    expect(o.x.uploadMedia).not.toHaveBeenCalled()
    expect(o.x.publishArticle).not.toHaveBeenCalled()
    const saved = await o.articles.get(result.slug!)
    expect(saved).toMatchObject({ status: 'dry-run', title: 'the inch', engine: expect.any(String), runId: 'run-two', runPath: 'data/backrooms/run-two.json' })
    expect(saved?.cover?.file).toBe('cover.png')
    expect(saved?.contentState?.entities).toHaveLength(3)
    expect(saved?.images.map((image) => image.file)).toEqual(['s1.png', 's2.png', 's3.png'])
    expect(o.articles.files.get(`${result.slug}/cover.png`)).toEqual(PNG)
    expect(result.slug).toBe('20260923-the-inch')
  })

  it('stops after the plan when asked, with nothing expanded', async () => {
    const o = options({ structureOnly: true })
    const result = await new ArticlePipeline(o).run()
    expect(result.outcome).toBe('structure')
    expect(result.structure?.title).toBe('the inch')
    expect((o.editor as ReturnType<typeof staged>).asked).toEqual(['structure', 'check'])
    expect((await o.articles.get(result.slug!))?.status).toBe('structure')
  })

  it('revises the plan once when the editor says no', async () => {
    let checks = 0
    const model = staged({
      check: () => JSON.stringify(checks++ === 0 ? { ok: false, problems: ['no turn'], fixes: ['add one'] } : { ok: true, problems: [], fixes: [] }),
    })
    const o = options({ writer: model, editor: model, fixer: model, structureOnly: true })
    const result = await new ArticlePipeline(o).run()
    expect(model.asked).toEqual(['structure', 'check', 'structure', 'check'])
    expect(result.structureCheck?.ok).toBe(true)
    const revision = (model.complete as ReturnType<typeof vi.fn>).mock.calls[2]![0] as { prompt: string }
    expect(revision.prompt).toContain('the editor said:\n- no turn\n- fix: add one')
  })

  it('takes a saved room instead of opening one', async () => {
    const o = options({ from: { run: { ...RUN, id: 'saved' }, thought: THOUGHT, path: 'data/backrooms/saved.json' }, noImages: true })
    const result = await new ArticlePipeline(o).run()
    expect(o.backrooms.run).not.toHaveBeenCalled()
    expect(o.seed).not.toHaveBeenCalled()
    expect(result.rooms).toEqual([{ index: 0, runId: 'saved', saved: 'data/backrooms/saved.json' }])
    expect(result.outcome).toBe('dry-run')
    expect(o.images!.generate).not.toHaveBeenCalled()
    expect(result.contentState?.entities).toEqual([])
  })

  it('skips when no room produced a thought, and when the wire is empty opens on the engine', async () => {
    const o = options({ seed: vi.fn(async () => undefined), backrooms: { run: vi.fn(async (): Promise<RunResult> => {
          const { thought: _thought, ...rest } = room('r', 20)
          return rest
        }) } })
    const result = await new ArticlePipeline(o).run()
    expect(result.outcome).toBe('skipped')
    expect(result.problems).toContain('no room produced a thought')
    expect(result.problems.join(' ')).toContain('the room opened on the engine alone')
    expect(o.backrooms.run.mock.calls[0]![0]).toMatchObject({ kind: 'text', source: 'lore' })
  })

  it('applies the proof-reader\'s fix and keeps its findings on the record', async () => {
    let reviews = 0
    const model = staged({
      proof: () =>
        JSON.stringify(
          reviews++ === 0
            ? { verdict: 'fix', findings: [{ section: 's1', pattern: '14 em dashes', quote: 'nobody asked it to', fix: 'nobody asked' }], voice: 'ok' }
            : { verdict: 'clean', findings: [], voice: 'ok' },
        ),
      fix: (prompt) => (prompt.split('the text:\n\n')[1] ?? '').replace(/nobody asked it to/g, 'nobody asked'),
    })
    const o = options({ writer: model, editor: model, fixer: model, noImages: true })
    const result = await new ArticlePipeline(o).run()
    expect(result.proof?.applied).toBe(true)
    expect(result.markdown).not.toContain('nobody asked it to')
    expect(result.proof?.after?.verdict).toBe('clean')
    expect((await o.articles.get(result.slug!))?.proof?.review.findings).toHaveLength(1)
  })

  it('holds the article when the safety gate holds a section, drawing nothing', async () => {
    const o = options({
      safety: {
        outbound: vi.fn(async (text: string) => ({
          decision: text.includes('section 2') ? { ok: false as const, category: 'hate', by: 'lexicon' as const, reason: 'a word' } : { ok: true as const },
          text,
          fixes: [],
        })) as never,
      },
    })
    const result = await new ArticlePipeline(o).run()
    expect(result.outcome).toBe('held')
    expect(o.images!.generate).not.toHaveBeenCalled()
    expect((await o.articles.get(result.slug!))?.status).toBe('held')
    // The title and dek, then each section on its own.
    const gated = (o.safety!.outbound as ReturnType<typeof vi.fn>).mock.calls.map((call) => (call[0] as string).split('\n')[0])
    expect(gated).toEqual(['the inch', '_a door, a ruler, and a ledger that balances in something nobody can name._', '## the gap', '## the ledger'])
  })

  it('skips when the article talks money or the critic reads it as risky', async () => {
    const priced = staged({ section: () => 'it is $0.002 now, nfa.' })
    const one = await new ArticlePipeline(options({ writer: priced, editor: priced, fixer: priced })).run()
    expect(one.outcome).toBe('skipped')
    expect(one.problems.join(' ')).toContain('talks money')

    const critic = { critique: vi.fn(async (): Promise<Critique> => ({ verdict: 'risky', score: -5, upside: 0, cost: -5, predicted: {} as Critique['predicted'], communityReply: 0, firstReply: '', disagreement: '', shareReason: '', riskReason: 'aimed at a person', fixes: [], summary: 'no' })) }
    const o = options({ critic })
    const two = await new ArticlePipeline(o).run()
    expect(two.outcome).toBe('skipped')
    expect(two.problems.join(' ')).toContain('risky')
    expect(o.images!.generate).not.toHaveBeenCalled()
  })

  it('publishes: uploads the pictures, sends the article, records the post', async () => {
    const o = options({ publish: true })
    const result = await new ArticlePipeline(o).run()
    expect(result.outcome).toBe('posted')
    expect(o.x.uploadMedia).toHaveBeenCalledTimes(4)
    const draft = o.x.publishArticle.mock.calls[0]![0] as { title: string; coverMediaId?: string; contentState: { entities: unknown[] } }
    expect(draft.title).toBe('the inch')
    expect(draft.coverMediaId).toBe('m1')
    expect(draft.contentState.entities).toHaveLength(3)
    expect(draft.contentState.entities[0]).toMatchObject({ value: { type: 'image', data: { mediaItems: [{ mediaId: 'm2' }], caption: 'the door' } } })
    expect(result.postId).toBe('9001')
    const saved = await o.articles.get(result.slug!)
    expect(saved).toMatchObject({ status: 'published', postId: '9001', xArticleId: 'a1', publishedAt: '2026-09-23T23:00:00.000Z' })
    expect(saved?.cover?.mediaId).toBe('m1')
    expect(saved?.images.map((image) => image.mediaId)).toEqual(['m2', 'm3', 'm4'])
    expect(o.captured).toHaveBeenCalledWith('article', '9001', expect.objectContaining({ title: 'the inch' }))
  })

  it('records a failure and rethrows when x refuses, posting nothing else', async () => {
    const o = options({ publish: true })
    o.x.publishArticle.mockRejectedValueOnce(new Error('X refused the article (403): not enrolled'))
    await expect(new ArticlePipeline(o).run()).rejects.toThrow(/403/)
    const [saved] = await o.articles.list({ all: true })
    expect(saved?.status).toBe('failed')
    expect((await o.articles.get(saved!.slug))?.error).toContain('403')
  })

  it('goes without pictures when the budget is spent, and never draws when told not to', async () => {
    const spent = options({ budget: new MediaBudget({ imagesPerDay: 1, videosPerDay: 0 }), publish: true })
    const result = await new ArticlePipeline(spent).run()
    expect(result.outcome).toBe('posted')
    expect(spent.images!.generate).toHaveBeenCalledTimes(1)
    expect(result.cover).toBeDefined()
    expect(result.images).toEqual([])
    expect(result.problems.join(' ')).toContain('ran out before the s1 picture')

    const none = options({ noImages: true })
    const budget = vi.spyOn(none.budget, 'take')
    await new ArticlePipeline(none).run()
    expect(budget).not.toHaveBeenCalled()
  })

  it('splits the markdown at the section headings', () => {
    expect(splitSections('_dek_\n\n## one\n\na\n\n## two\n\nb')).toEqual(['_dek_', '## one\n\na', '## two\n\nb'])
  })
})
