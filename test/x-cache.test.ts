import { describe, expect, it, vi } from 'vitest'
import { DEFAULT_X_CACHE_TTL, InMemoryXCache, cached, type XCacheStore } from '../src/clients/x-cache.js'
import type { XPost, XSearchOptions } from '../src/types.js'

/**
 * X bills every post a read returns. The cache exists so nothing is bought
 * twice: a post seen through any read is served from the store afterwards,
 * a list asked for twice in one window is one request, and the store
 * failing costs a hit, never a read.
 */

const post = (id: string, text = `post ${id}`): XPost => ({ id, text, authorId: 'u1', conversationId: id })

function fakeClient() {
  return {
    searchPosts: vi.fn(async (_query: string, _options?: XSearchOptions) => [post('1'), post('2')]),
    getPost: vi.fn(async (id: string) => (id === '404' ? undefined : post(id))),
    getConversation: vi.fn(async (id: string, _max?: number) => [post(`${id}-a`), post(`${id}-b`)]),
    trends: vi.fn(async (_options?: { woeid?: number; max?: number }) => [{ name: 'tiny' }]),
    postMetrics: vi.fn(async (ids: string[]) =>
      ids.map((id) => ({ id, impressions: 1, likes: 0, replies: 0, reposts: 0, quotes: 0 })),
    ),
    followsAccount: vi.fn(async (id: string) => (id === 'unknown' ? undefined : true)),
    createPost: vi.fn(async (_text: string) => ({ id: 'new' })),
  }
}

describe('the X read cache', () => {
  it('serves a post from the store after the first read', async () => {
    const x = fakeClient()
    const c = cached(x, new InMemoryXCache())
    expect(await c.getPost('7')).toEqual(post('7'))
    expect(await c.getPost('7')).toEqual(post('7'))
    expect(x.getPost).toHaveBeenCalledTimes(1)
  })

  it('remembers every post a search or a conversation brought back, so getPost is free', async () => {
    const x = fakeClient()
    const c = cached(x, new InMemoryXCache())
    await c.searchPosts('$TINY')
    await c.getConversation('9', 25)
    expect(await c.getPost('1')).toEqual(post('1'))
    expect(await c.getPost('9-b')).toEqual(post('9-b'))
    expect(x.getPost).not.toHaveBeenCalled()
  })

  it('keeps a search only for its window, and keys it on the query and options', async () => {
    let now = 1_000_000
    const x = fakeClient()
    const c = cached(x, new InMemoryXCache(), { now: () => now, ttl: { search: 60_000 } })
    await c.searchPosts('$TINY', { maxResults: 10 })
    await c.searchPosts('$TINY', { maxResults: 10 })
    expect(x.searchPosts).toHaveBeenCalledTimes(1)
    await c.searchPosts('$TINY', { maxResults: 20 })
    expect(x.searchPosts).toHaveBeenCalledTimes(2)
    now += 60_001
    await c.searchPosts('$TINY', { maxResults: 10 })
    expect(x.searchPosts).toHaveBeenCalledTimes(3)
  })

  it('rounds a moving time window to the search TTL so an hourly job asks once per window', async () => {
    let now = 10 * 60_000
    const x = fakeClient()
    const c = cached(x, new InMemoryXCache(), { now: () => now, ttl: { search: 10 * 60_000 } })
    const window = () => ({ startTime: new Date(now - 3_600_000).toISOString(), endTime: new Date(now - 60_000).toISOString() })
    await c.searchPosts('$TINY', window())
    now += 30_000
    await c.searchPosts('$TINY', window())
    expect(x.searchPosts).toHaveBeenCalledTimes(1)
  })

  it('remembers that a post was missing, but only briefly', async () => {
    let now = 0
    const x = fakeClient()
    const c = cached(x, new InMemoryXCache(), { now: () => now })
    expect(await c.getPost('404')).toBeUndefined()
    expect(await c.getPost('404')).toBeUndefined()
    expect(x.getPost).toHaveBeenCalledTimes(1)
    now += DEFAULT_X_CACHE_TTL.missing + 1
    await c.getPost('404')
    expect(x.getPost).toHaveBeenCalledTimes(2)
  })

  it('shares one flight between callers asking the same thing at once', async () => {
    const x = fakeClient()
    const c = cached(x, new InMemoryXCache())
    await Promise.all([c.getPost('3'), c.getPost('3'), c.getConversation('3'), c.getConversation('3')])
    expect(x.getPost).toHaveBeenCalledTimes(1)
    expect(x.getConversation).toHaveBeenCalledTimes(1)
  })

  it('caches trends, own metrics (order-insensitive) and a known follow, not an unknown one', async () => {
    const x = fakeClient()
    const c = cached(x, new InMemoryXCache())
    await c.trends()
    await c.trends({ woeid: 1 })
    expect(x.trends).toHaveBeenCalledTimes(1)
    await c.postMetrics(['a', 'b'])
    await c.postMetrics(['b', 'a'])
    expect(x.postMetrics).toHaveBeenCalledTimes(1)
    await c.followsAccount('u2')
    await c.followsAccount('u2')
    expect(x.followsAccount).toHaveBeenCalledTimes(1)
    await c.followsAccount('unknown')
    await c.followsAccount('unknown')
    expect(x.followsAccount).toHaveBeenCalledTimes(3)
  })

  it('goes to X when the store is broken, and never touches writes', async () => {
    const x = fakeClient()
    const broken: XCacheStore = {
      get: async () => { throw new Error('mongo down') },
      set: async () => { throw new Error('mongo down') },
      close: async () => {},
    }
    const warn = vi.fn()
    const c = cached(x, broken, { logger: { warn } })
    expect(await c.getPost('5')).toEqual(post('5'))
    expect(await c.getPost('5')).toEqual(post('5'))
    expect(x.getPost).toHaveBeenCalledTimes(2)
    expect(warn).toHaveBeenCalled()
    expect(await c.createPost('hi')).toEqual({ id: 'new' })
    expect(x.createPost).toHaveBeenCalledTimes(1)
  })
})
