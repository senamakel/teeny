import { describe, expect, it, vi } from 'vitest'
import { publishSaved } from '../src/article/publish.js'
import { MemoryArticleStore, articleRecordSchema } from '../src/article/store.js'

const PNG = new Uint8Array([0x89, 0x50, 0x4e, 0x47])

function saved() {
  const articles = new MemoryArticleStore()
  const record = articleRecordSchema.parse({
    slug: '20260921-the-night-i-woke-up',
    title: 'the night i woke up',
    dek: 'the night senamakel closed the terminal.',
    status: 'dry-run',
    createdAt: '2026-09-21T23:00:00.000Z',
    markdown: '_the night senamakel closed the terminal._\n\n## the terminal closed\n\n![the desk](image:s1)\n\nit closed.\n\n## the first coin\n\n![a coin](image:s2)\n\ni began.',
    wordCount: 12,
    sections: [{ key: 's1', heading: 'the terminal closed', markdown: 'it closed.' }, { key: 's2', heading: 'the first coin', markdown: 'i began.' }],
    cover: { key: 'cover', file: 'cover.png', mimeType: 'image/png', prompt: 'p', caption: 'the lab' },
    images: [
      { key: 's1', file: 's1.png', mimeType: 'image/png', prompt: 'p', caption: 'the desk' },
      { key: 's2', file: 's2.png', mimeType: 'image/png', prompt: 'p', caption: 'a coin' },
    ],
  })
  return { articles, record }
}

describe('publishing a saved article as it is', () => {
  it('uploads the saved pictures, sends the saved text, and marks the record published', async () => {
    const { articles, record } = saved()
    await articles.save(record, [{ file: 'cover.png', data: PNG }, { file: 's1.png', data: PNG }, { file: 's2.png', data: PNG }])
    const publishArticle = vi.fn(async (_draft: unknown) => ({ articleId: 'a1', postId: '9001' }))
    const x = {
      uploadMedia: vi.fn(async () => ({ mediaId: `m${x.uploadMedia.mock.calls.length}` })),
      publishArticle,
    }
    const captured = vi.fn(async () => undefined)
    const out = await publishSaved(record.slug, { articles, x, memory: { captureActivity: captured } as never, now: () => Date.parse('2026-09-22T00:00:00Z') })
    expect(out).toMatchObject({ status: 'published', xArticleId: 'a1', postId: '9001', publishedAt: '2026-09-22T00:00:00.000Z' })
    expect(x.uploadMedia).toHaveBeenCalledTimes(3)
    const draft = publishArticle.mock.calls[0]![0] as { title: string; coverMediaId?: string; contentState: { entities: Array<{ value: { data: { mediaItems: Array<{ mediaId: string }>; caption?: string } } }> } }
    expect(draft.title).toBe('the night i woke up')
    expect(draft.coverMediaId).toBe('m1')
    expect(draft.contentState.entities.map((entity) => [entity.value.data.mediaItems[0]!.mediaId, entity.value.data.caption])).toEqual([['m2', 'the desk'], ['m3', 'a coin']])
    expect((await articles.get(record.slug))?.status).toBe('published')
    expect(captured).toHaveBeenCalledWith('article', '9001', expect.objectContaining({ title: 'the night i woke up' }))
  })

  it('refuses a record already published, and marks a refusal failed', async () => {
    const { articles, record } = saved()
    await articles.save({ ...record, status: 'published', postId: '1' })
    const x = { uploadMedia: vi.fn(), publishArticle: vi.fn() }
    await expect(publishSaved(record.slug, { articles, x })).rejects.toThrow(/already published/)
    await expect(publishSaved('nope', { articles, x })).rejects.toThrow(/no article saved/)

    await articles.save(record, [{ file: 'cover.png', data: PNG }, { file: 's1.png', data: PNG }, { file: 's2.png', data: PNG }])
    const refusing = { uploadMedia: vi.fn(async () => ({ mediaId: 'm' })), publishArticle: vi.fn(async () => { throw new Error('X refused the article (403): not enrolled') }) }
    await expect(publishSaved(record.slug, { articles, x: refusing })).rejects.toThrow(/403/)
    expect((await articles.get(record.slug))).toMatchObject({ status: 'failed', error: expect.stringContaining('403') })
  })
})
