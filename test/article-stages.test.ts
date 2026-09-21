import { describe, expect, it, vi } from 'vitest'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type { TextModel } from '../src/media/reference.js'
import {
  loadHumanizer,
  proofFixPrompt,
  proofPrompt,
  proofread,
  stripFrontmatter,
  structurallySame,
  unfence,
} from '../src/article/proofread.js'
import {
  articleStyle,
  assembleMarkdown,
  checkStructurePrompt,
  dekOf,
  withoutDek,
  normaliseStructure,
  renderStructure,
  sectionPrompt,
  stripSectionExtras,
  structurePrompt,
  structureSchema,
  wordCount,
  type ArticleMaterial,
} from '../src/article/stages.js'
import { RUN, STRUCTURE, THOUGHT } from './fixtures/article.js'
import { FileArticleStore, MemoryArticleStore, articleRecordSchema, freeSlug, slugFor, type ArticleRecord } from '../src/article/store.js'

const MATERIAL: ArticleMaterial = { run: RUN, thought: THOUGHT, engine: 'the box: teeny is being polite about leaving.', lore: 'the goals:\n- mint my own currency' }

describe('the structure', () => {
  it('is planned from the room, the engine and the surfaced lore only', () => {
    const { system, prompt } = structurePrompt(MATERIAL, { sectionsMax: 4, words: 1500 })
    expect(system).toContain('lore only')
    expect(prompt).toContain('tonight the story is about the box')
    expect(prompt).toContain('mint my own currency')
    expect(prompt).toContain('the inch is twelve millimetres')
    expect(prompt).toContain('about 1500 words')
    expect(prompt).toContain('three to 4 sections')
  })

  it('gets positional keys and word targets that add up', () => {
    const out = normaliseStructure(STRUCTURE, 1500)
    expect(out.sections.map((section) => section.key)).toEqual(['s1', 's2', 's3'])
    expect(out.sections.map((section) => section.words)).toEqual([600, 600, 300])
    expect(structureSchema.parse(out)).toEqual(out)
  })

  it('is checked against the lore and rendered for a reader', () => {
    const { prompt } = checkStructurePrompt(STRUCTURE, MATERIAL)
    expect(prompt).toContain('title: the inch')
    expect(prompt).toContain('1. the gap (400 words)')
    expect(prompt).toContain('- teeny sees the gap')
    expect(prompt).toContain('mint my own currency')
    expect(renderStructure(STRUCTURE)).toContain('closing: the lock')
  })
})

describe('the sections', () => {
  it('are written one at a time with the earlier ones in view', () => {
    const structure = normaliseStructure(STRUCTURE, 1000)
    const first = sectionPrompt(structure, 0, [], MATERIAL)
    expect(first.prompt).toContain('write section 1 of 3: "the gap"')
    expect(first.prompt).toContain('about 400 words')
    expect(first.prompt).not.toContain('already written')
    const second = sectionPrompt(structure, 1, ['the gap was an inch.'], MATERIAL)
    expect(second.prompt).toContain('already written')
    expect(second.prompt).toContain('## the gap\nthe gap was an inch.')
    expect(second.prompt).toContain('- crayon on the desk')
  })

  it('are assembled under their headings with the picture right under each', () => {
    const structure = normaliseStructure(STRUCTURE, 1000)
    const markdown = assembleMarkdown(structure, ['## the gap\n\nbody one', 'body two\n\n![x](image:nope)', 'body three'])
    expect(markdown).toBe(
      [
        '_a door, a ruler, and a ledger that balances in something nobody can name._',
        '',
        '## the gap',
        '',
        '![the door](image:s1)',
        '',
        'body one',
        '',
        '## the ledger',
        '',
        '![crayon](image:s2)',
        '',
        'body two',
        '',
        '## the lock',
        '',
        '![the lock](image:s3)',
        '',
        'body three',
      ].join('\n'),
    )
    expect(stripSectionExtras('### heading\ntext\n![a](b)')).toBe('text')
    expect(dekOf(markdown)).toBe('a door, a ruler, and a ledger that balances in something nobody can name.')
    expect(withoutDek(markdown)).toMatch(/^## the gap/)
    expect(dekOf('## no dek')).toBeUndefined()
    expect(withoutDek('## no dek')).toBe('## no dek')
    expect(wordCount(markdown)).toBe(26)
  })
})

describe('the style pass', () => {
  it('cuts urls, handles and hashtags, keeps $TINY, and reports a price', () => {
    const out = articleStyle('see https://x.com/a and @someone about #tiny and $TINY\nit is $0.002 now, nfa\nfine line')
    expect(out.text).toBe('see and about and $TINY\nit is $0.002 now, nfa\nfine line')
    expect(out.fixes).toEqual(['cut 1 url(s)', 'cut 1 handle(s)', 'cut 1 hashtag(s)'])
    expect(out.priced).toEqual(['it is $0.002 now, nfa'])
  })

  it('leaves an email-less clean text alone', () => {
    expect(articleStyle('the kettle boils. nobody asked it to.')).toEqual({ text: 'the kettle boils. nobody asked it to.', fixes: [], priced: [] })
  })
})

describe('the proof-reader', () => {
  const skill = '# Humanizer\n\n### 14. Em dashes\n\nToo many.'
  const draft = '_dek_\n\n## the gap\n\n![the door](image:s1)\n\nthe door — it was open — an inch.\n\n## the lock\n\n![the lock](image:s2)\n\nthe lock sat there.'
  const clean = { verdict: 'clean', findings: [], voice: 'yes' }
  const fixIt = { verdict: 'fix', findings: [{ section: 's1', pattern: '14 em dashes', quote: 'the door — it was open — an inch.', fix: 'the door was open an inch.' }], voice: 'yes' }
  const model = (answers: { review: unknown[]; fix?: string }): { reviewer: TextModel; fixer: TextModel; calls: string[] } => {
    const calls: string[] = []
    const reviews = [...answers.review]
    return {
      calls,
      reviewer: { complete: vi.fn(async () => { calls.push('review'); return JSON.stringify(reviews.shift() ?? clean) }) },
      fixer: { complete: vi.fn(async () => { calls.push('fix'); return answers.fix ?? draft }) },
    }
  }

  it('reads with the skill as the checklist and the sample for the voice', () => {
    const { system, prompt } = proofPrompt(skill, draft, 'the door was one inch open.')
    expect(system).toContain('### 14. Em dashes')
    expect(system).toContain('proof-reading, not rewriting')
    expect(prompt).toContain('the door was one inch open.')
    expect(prompt).toContain('"verdict":"clean"|"fix"')
    const fix = proofFixPrompt(skill, draft, fixIt.findings, 'sample')
    expect(fix.prompt).toContain('[s1] 14 em dashes: "the door — it was open — an inch." → the door was open an inch.')
    expect(fix.system).toContain('keep every `## ` heading')
  })

  it('skips the fix when the review is clean', async () => {
    const m = model({ review: [clean] })
    const out = await proofread(m, { skill, markdown: draft, sample: 's' })
    expect(out).toEqual({ markdown: draft, review: clean, applied: false })
    expect(m.calls).toEqual(['review'])
  })

  it('applies the findings and reads again', async () => {
    const fixed = draft.replace('the door — it was open — an inch.', 'the door was open an inch.')
    const m = model({ review: [fixIt, clean], fix: `\`\`\`markdown\n${fixed}\n\`\`\`` })
    const out = await proofread(m, { skill, markdown: draft, sample: 's' })
    expect(out.applied).toBe(true)
    expect(out.markdown).toBe(fixed)
    expect(out.after).toEqual(clean)
    expect(m.calls).toEqual(['review', 'fix', 'review'])
  })

  it('refuses a fix that changed the shape or said there was nothing to fix', async () => {
    for (const [fix, reason] of [
      ['Nothing to fix.', 'the fix said there was nothing to fix'],
      [draft.replace('## the lock', '## the key'), 'the headings changed'],
      [draft.replace('![the lock](image:s2)\n\n', ''), 'an image placeholder moved or went missing'],
      ['_dek_\n\n## the gap\n\n![the door](image:s1)\n\n## the lock\n\n![the lock](image:s2)', /of the length$/],
    ] as const) {
      const m = model({ review: [fixIt], fix })
      const out = await proofread(m, { skill, markdown: draft, sample: 's' })
      expect(out.applied).toBe(false)
      expect(out.markdown).toBe(draft)
      expect(out.refused).toMatch(reason)
      expect(m.calls).toEqual(['review', 'fix'])
    }
    expect(structurallySame(draft, draft)).toBeUndefined()
    expect(unfence('```md\nx\n```')).toBe('x')
  })

  it('loads the skill without its frontmatter', () => {
    const dir = mkdtempSync(join(tmpdir(), 'humanizer-'))
    try {
      writeFileSync(join(dir, 'SKILL.md'), '---\nname: humanizer\ndescription: x\n---\n\n# Humanizer\n\nbody\n')
      expect(loadHumanizer(join(dir, 'SKILL.md'))).toBe('# Humanizer\n\nbody')
      expect(stripFrontmatter('no frontmatter')).toBe('no frontmatter')
    } finally {
      rmSync(dir, { recursive: true, force: true })
    }
  })
})

describe('the store', () => {
  const record = (over: Partial<ArticleRecord>): ArticleRecord =>
    articleRecordSchema.parse({ slug: '20260921-the-inch', title: 'the inch', dek: 'a door', status: 'dry-run', createdAt: '2026-09-21T12:00:00.000Z', ...over })

  it('slugs a title by the day and finds a free one', async () => {
    expect(slugFor(new Date('2026-09-21T23:00:00Z'), 'The Inch: a door, a ruler & a ledger')).toBe('20260921-the-inch-a-door-a-ruler-a-ledger')
    expect(slugFor(new Date('2026-09-21T23:00:00Z'), '!!!')).toBe('20260921')
    const store = new MemoryArticleStore()
    await store.save(record({}))
    expect(await freeSlug(store, '20260921-the-inch')).toBe('20260921-the-inch-2')
    expect(await freeSlug(store, '20260921-other')).toBe('20260921-other')
  })

  it('writes the record and the pictures to a directory and reads them back', async () => {
    const dir = mkdtempSync(join(tmpdir(), 'articles-'))
    try {
      const store = new FileArticleStore(join(dir, 'articles'))
      expect(await store.list({ all: true })).toEqual([])
      const png = new Uint8Array([0x89, 0x50, 0x4e, 0x47])
      await store.save(
        record({ cover: { key: 'cover', file: 'cover.png', mimeType: 'image/png', prompt: 'p', caption: '' }, wordCount: 3, engine: 'the box', structure: STRUCTURE }),
        [{ file: 'cover.png', data: png }],
      )
      await store.save(record({ slug: '20260924-later', status: 'published', publishedAt: '2026-09-24T23:00:00.000Z', postId: '9', createdAt: '2026-09-24T22:00:00.000Z' }))
      expect((await store.list()).map((item) => item.slug)).toEqual(['20260924-later'])
      expect((await store.list({ all: true })).map((item) => item.slug)).toEqual(['20260924-later', '20260921-the-inch'])
      expect((await store.list({ all: true }))[1]).toMatchObject({ cover: 'cover.png', wordCount: 3, status: 'dry-run' })
      expect((await store.get('20260921-the-inch'))?.structure?.title).toBe('the inch')
      expect(await store.get('../etc')).toBeUndefined()
      expect(await store.media('20260921-the-inch', 'cover.png')).toEqual({ data: Buffer.from(png), mimeType: 'image/png' })
      expect(await store.media('20260921-the-inch', '../article.json')).toBeUndefined()
      expect(await store.media('20260921-the-inch', 'nope.png')).toBeUndefined()
      expect(await store.latestPublishedAt()).toBe('2026-09-24T23:00:00.000Z')
      expect(await store.lastEngine()).toBe('the box')
      await expect(store.save(record({}), [{ file: 'evil.sh', data: png }])).rejects.toThrow(/not an image/)
    } finally {
      rmSync(dir, { recursive: true, force: true })
    }
  })
})
