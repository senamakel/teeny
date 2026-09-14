import { describe, expect, it, vi } from 'vitest'
import { createGiphyTools } from '../src/agent/tools/giphy.js'
import { createXTools } from '../src/agent/tools/x.js'
import type { Gif, GifSource, XPost, XTools } from '../src/types.js'

const GIF: Gif = {
  id: 'abc123',
  title: 'tiny waving',
  pageUrl: 'https://giphy.com/gifs/abc123',
  mediaUrl: 'https://media.giphy.com/abc123.gif',
  mimeType: 'image/gif',
}

function gifSource(overrides: Partial<GifSource> = {}): GifSource {
  return {
    search: vi.fn().mockResolvedValue([GIF]),
    get: vi.fn().mockResolvedValue(GIF),
    download: vi
      .fn()
      .mockResolvedValue({ data: new Uint8Array([1, 2, 3]), mimeType: 'image/gif' }),
    ...overrides,
  }
}

function xTools(): XTools {
  const post: XPost = { id: '10', text: 'hi' }
  return {
    searchPosts: vi.fn().mockResolvedValue([post]),
    getPost: vi.fn().mockResolvedValue(post),
    getConversation: vi.fn().mockResolvedValue([post]),
    createPost: vi.fn().mockResolvedValue({ id: '77' }),
    uploadMedia: vi.fn().mockResolvedValue({ mediaId: 'media-1' }),
    likePost: vi.fn().mockResolvedValue(undefined),
    postMetrics: vi.fn().mockResolvedValue([]),
  }
}

function byName(tools: Array<{ name: string; invoke: (input: never) => Promise<unknown> }>) {
  return new Map(tools.map((tool) => [tool.name, tool]))
}

describe('find_gif', () => {
  it('searches the account channel by default', async () => {
    const gifs = gifSource()
    const tools = byName(createGiphyTools(gifs, 'TinyHumansAI'))
    const output = await tools.get('find_gif')!.invoke({ query: 'waving' } as never)

    expect(gifs.search).toHaveBeenCalledWith('waving', { channelOnly: true })
    expect(output).toContain('id: abc123')
    expect(output).toContain('tiny waving')
  })

  it('can be told to search beyond the channel', async () => {
    const gifs = gifSource()
    const tools = byName(createGiphyTools(gifs, 'TinyHumansAI'))
    await tools.get('find_gif')!.invoke({ query: 'waving', searchAllOfGiphy: true } as never)

    expect(gifs.search).toHaveBeenCalledWith('waving', { channelOnly: false })
  })

  it('names the channel when nothing matched inside it', async () => {
    const gifs = gifSource({ search: vi.fn().mockResolvedValue([]) })
    const tools = byName(createGiphyTools(gifs, 'TinyHumansAI'))
    const output = await tools.get('find_gif')!.invoke({ query: 'waving' } as never)

    expect(output).toContain('@TinyHumansAI')
    expect(output).toContain('searchAllOfGiphy')
  })
})

describe('x_post with a GIF', () => {
  it('resolves the id, uploads the media, and attaches it', async () => {
    const x = xTools()
    const gifs = gifSource()
    const tools = byName(
      createXTools(x, { dryRun: false, gifs: { source: gifs, channel: 'TinyHumansAI' } }),
    )
    const output = await tools.get('x_post')!.invoke({ text: 'gm', gifId: 'abc123' } as never)

    expect(gifs.get).toHaveBeenCalledWith('abc123')
    expect(x.uploadMedia).toHaveBeenCalledWith({
      data: new Uint8Array([1, 2, 3]),
      mimeType: 'image/gif',
    })
    expect(x.createPost).toHaveBeenCalledWith('gm', { mediaIds: ['media-1'] })
    expect(output).toBe('Published post 77 with a GIF.')
  })

  it('refuses an id Giphy does not know', async () => {
    const x = xTools()
    const gifs = gifSource({ get: vi.fn().mockResolvedValue(undefined) })
    const tools = byName(
      createXTools(x, { dryRun: false, gifs: { source: gifs, channel: 'TinyHumansAI' } }),
    )
    const output = await tools.get('x_post')!.invoke({ text: 'gm', gifId: 'nope' } as never)

    expect(output).toContain('Refused')
    expect(x.createPost).not.toHaveBeenCalled()
  })

  it('refuses a GIF when no source is configured', async () => {
    const x = xTools()
    const tools = byName(createXTools(x, { dryRun: false }))
    const output = await tools.get('x_post')!.invoke({ text: 'gm', gifId: 'abc123' } as never)

    expect(output).toContain('no GIF source is configured')
    expect(x.createPost).not.toHaveBeenCalled()
  })
})
