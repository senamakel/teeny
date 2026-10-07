import { describe, expect, it, vi } from 'vitest'
import { MemoryArticleStore, articleRecordSchema } from '../src/article/store.js'
import { buildServer } from '../src/server.js'
import type { Memory } from '../src/types.js'

function memory(): Memory {
  return {
    captureActivity: vi.fn().mockResolvedValue(undefined),
    captureMention: vi.fn().mockResolvedValue(undefined),
    recall: vi.fn().mockResolvedValue(''),
    captureReply: vi.fn().mockResolvedValue(undefined),
  }
}

describe('GET /articles', () => {
  it('serves the published articles, every rehearsal on request, one piece without the operator detail, and its pictures', async () => {
    const articles = new MemoryArticleStore()
    const png = new Uint8Array([0x89, 0x50, 0x4e, 0x47])
    await articles.save(
      articleRecordSchema.parse({
        slug: '20260923-the-inch',
        title: 'the inch',
        dek: 'a door',
        status: 'published',
        createdAt: '2026-09-23T23:00:00.000Z',
        publishedAt: '2026-09-23T23:05:00.000Z',
        postId: '9001',
        markdown: '## the gap\n\na',
        wordCount: 3,
        cover: { key: 'cover', file: 'cover.png', mimeType: 'image/png', prompt: 'p', caption: '' },
        problems: ['kept a picture the check did not like'],
        critique: 'Verdict: FLAT',
      }),
      [{ file: 'cover.png', data: png }],
    )
    await articles.save(articleRecordSchema.parse({ slug: '20260920-rehearsal', title: 'r', dek: 'd', status: 'dry-run', createdAt: '2026-09-20T23:00:00.000Z' }))
    const app = buildServer({ memory: memory(), consumerSecret: 'secret', articles })

    const list = await app.inject({ method: 'GET', url: '/articles' })
    expect(list.statusCode).toBe(200)
    expect(list.json()).toMatchObject({ source: 'live', articles: [{ slug: '20260923-the-inch', cover: 'cover.png', postId: '9001' }] })
    expect(list.json().articles).toHaveLength(1)
    expect((await app.inject({ method: 'GET', url: '/articles?all=1' })).json().articles).toHaveLength(2)

    const one = await app.inject({ method: 'GET', url: '/articles/20260923-the-inch' })
    expect(one.statusCode).toBe(200)
    expect(one.json()).toMatchObject({ title: 'the inch', markdown: '## the gap\n\na' })
    expect(one.json()).not.toHaveProperty('problems')
    expect(one.json()).not.toHaveProperty('critique')
    expect((await app.inject({ method: 'GET', url: '/articles/20260923-the-inch?all=1' })).json().problems).toHaveLength(1)
    expect((await app.inject({ method: 'GET', url: '/articles/nope' })).statusCode).toBe(404)

    const cover = await app.inject({ method: 'GET', url: '/articles/20260923-the-inch/media/cover.png' })
    expect(cover.statusCode).toBe(200)
    expect(cover.headers['content-type']).toBe('image/png')
    expect(cover.headers['cache-control']).toBe('public, max-age=86400')
    expect(cover.rawPayload).toEqual(Buffer.from(png))
    expect((await app.inject({ method: 'GET', url: '/articles/20260923-the-inch/media/other.png' })).statusCode).toBe(404)
    await app.close()
  })

  it('says so when no store is configured', async () => {
    const app = buildServer({ memory: memory(), consumerSecret: 'secret' })
    expect((await app.inject({ method: 'GET', url: '/articles' })).statusCode).toBe(503)
    await app.close()
  })
})
