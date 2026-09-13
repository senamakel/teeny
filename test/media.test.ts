import { describe, expect, it, vi } from 'vitest'
import { OpenAIImageGenerator } from '../src/media/images.js'
import { MediaBudget, MediaStore } from '../src/media/store.js'
import { OpenAIVideoGenerator } from '../src/media/videos.js'
import { OpenAIVisionReader, isFetchableImageUrl, sniff } from '../src/media/vision.js'

const PNG = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0])

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } })
}

describe('MediaStore', () => {
  it('hands out ids by kind, keeps the bytes, and forgets them after the TTL', () => {
    let now = 1_000_000
    const store = new MediaStore({ ttlMs: 60_000, now: () => now })
    const image = store.put({ kind: 'image', prompt: 'a', data: new Uint8Array([1]), mimeType: 'image/png' })
    const video = store.put({ kind: 'video', prompt: 'b', data: new Uint8Array([2]), mimeType: 'video/mp4' })

    expect(image.id).toMatch(/^img_/)
    expect(video.id).toMatch(/^vid_/)
    expect(store.get(image.id)?.data).toEqual(new Uint8Array([1]))

    now += 61_000
    expect(store.get(image.id)).toBeUndefined()
    expect(store.list()).toEqual([])
  })

  it('drops the oldest when full', () => {
    const store = new MediaStore({ maxItems: 2 })
    const first = store.put({ kind: 'image', prompt: 'a', data: new Uint8Array([1]), mimeType: 'image/png' })
    store.put({ kind: 'image', prompt: 'b', data: new Uint8Array([2]), mimeType: 'image/png' })
    expect(store.get(first.id)).toBeUndefined()
    expect(store.list()).toHaveLength(1)
  })
})

describe('MediaBudget', () => {
  it('counts images and videos apart, and starts over at UTC midnight', () => {
    let now = Date.UTC(2026, 8, 14, 23, 59)
    const budget = new MediaBudget({ imagesPerDay: 2, videosPerDay: 1 }, () => now)

    expect(budget.take('image')).toBe(true)
    expect(budget.take('image')).toBe(true)
    expect(budget.take('image')).toBe(false)
    expect(budget.remaining('image')).toBe(0)
    expect(budget.take('video')).toBe(true)
    expect(budget.take('video')).toBe(false)

    now += 2 * 60_000
    expect(budget.remaining('image')).toBe(2)
    expect(budget.take('video')).toBe(true)
  })

  it('is closed at zero', () => {
    const budget = new MediaBudget({ imagesPerDay: 0, videosPerDay: 0 })
    expect(budget.take('image')).toBe(false)
  })
})

describe('isFetchableImageUrl', () => {
  it('allows public https names and nothing that points back at the box', () => {
    expect(isFetchableImageUrl('https://pbs.twimg.com/media/a.jpg')).toBe(true)
    expect(isFetchableImageUrl('http://pbs.twimg.com/media/a.jpg')).toBe(false)
    expect(isFetchableImageUrl('https://localhost/a.png')).toBe(false)
    expect(isFetchableImageUrl('https://127.0.0.1/a.png')).toBe(false)
    expect(isFetchableImageUrl('https://169.254.169.254/latest/meta-data')).toBe(false)
    expect(isFetchableImageUrl('https://[::1]/a.png')).toBe(false)
    expect(isFetchableImageUrl('https://mongo.internal/a.png')).toBe(false)
    expect(isFetchableImageUrl('file:///etc/passwd')).toBe(false)
    expect(isFetchableImageUrl('not a url')).toBe(false)
  })
})

describe('sniff', () => {
  it('reads the type from the first bytes', () => {
    expect(sniff(PNG)).toBe('image/png')
    expect(sniff(new Uint8Array([0xff, 0xd8, 0xff, 0xe0, 0, 0, 0, 0, 0, 0, 0, 0]))).toBe('image/jpeg')
    expect(sniff(new Uint8Array([0x47, 0x49, 0x46, 0x38, 0x39, 0x61, 0, 0, 0, 0, 0, 0]))).toBe('image/gif')
    expect(sniff(new TextEncoder().encode('RIFF....WEBPVP8 '))).toBe('image/webp')
    expect(sniff(new TextEncoder().encode('hello world!'))).toBeUndefined()
  })
})

describe('OpenAIVisionReader', () => {
  it('inlines the bytes as a data URL and returns the description', async () => {
    const fetchMock = vi.fn().mockResolvedValue(json({ choices: [{ message: { content: '  a cat  ' } }] }))
    const reader = new OpenAIVisionReader({
      baseUrl: 'https://llm.example/v1/',
      apiKey: 'k',
      model: 'vision',
      fetch: fetchMock as unknown as typeof fetch,
    })

    await expect(reader.describe({ data: PNG, mimeType: 'image/png' }, 'what is it?')).resolves.toBe('a cat')

    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit]
    expect(url).toBe('https://llm.example/v1/chat/completions')
    const body = JSON.parse(init.body as string) as {
      model: string
      messages: Array<{ role: string; content: unknown }>
    }
    expect(body.model).toBe('vision')
    const user = body.messages[1]?.content as Array<{ type: string; text?: string; image_url?: { url: string } }>
    expect(user[0]).toEqual({ type: 'text', text: 'what is it?' })
    expect(user[1]?.image_url?.url).toMatch(/^data:image\/png;base64,iVBORw0KGgo/)
  })

  it('fetches a URL itself, checks it is an image, and refuses a private one', async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(new Response(PNG, { status: 200, headers: { 'content-type': 'image/png' } }))
      .mockResolvedValueOnce(json({ choices: [{ message: { content: 'a chart' } }] }))
    const reader = new OpenAIVisionReader({
      baseUrl: 'https://llm.example/v1',
      apiKey: 'k',
      model: 'vision',
      fetch: fetchMock as unknown as typeof fetch,
    })

    await expect(reader.describe({ url: 'https://pbs.twimg.com/media/a.png' })).resolves.toBe('a chart')
    expect(fetchMock).toHaveBeenCalledTimes(2)

    await expect(reader.describe({ url: 'https://127.0.0.1/secret.png' })).rejects.toThrow(/refusing/)
    expect(fetchMock).toHaveBeenCalledTimes(2)
  })

  it('rejects a URL that does not serve an image', async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValue(new Response('<html>', { status: 200, headers: { 'content-type': 'text/html' } }))
    const reader = new OpenAIVisionReader({
      baseUrl: 'https://llm.example/v1',
      apiKey: 'k',
      model: 'vision',
      fetch: fetchMock as unknown as typeof fetch,
    })
    await expect(reader.describe({ url: 'https://example.com/page' })).rejects.toThrow(/did not return an image/)
  })

  it('surfaces the provider error', async () => {
    const fetchMock = vi.fn().mockResolvedValue(json({ error: { message: 'model cannot see' } }, 400))
    const reader = new OpenAIVisionReader({
      baseUrl: 'https://llm.example/v1',
      apiKey: 'k',
      model: 'text-only',
      fetch: fetchMock as unknown as typeof fetch,
    })
    await expect(reader.describe({ data: PNG, mimeType: 'image/png' })).rejects.toThrow(/400.*model cannot see/)
  })
})

describe('OpenAIImageGenerator', () => {
  it('asks gpt-image for PNG bytes in the model\'s own size vocabulary', async () => {
    const fetchMock = vi.fn().mockResolvedValue(json({ data: [{ b64_json: Buffer.from(PNG).toString('base64') }] }))
    const images = new OpenAIImageGenerator({
      baseUrl: 'https://api.example/v1',
      apiKey: 'k',
      model: 'gpt-image-1',
      quality: 'medium',
      fetch: fetchMock as unknown as typeof fetch,
    })

    const result = await images.generate({ prompt: 'a tiny robot', aspect: 'landscape' })
    expect(result).toEqual({ data: PNG, mimeType: 'image/png' })
    const body = JSON.parse((fetchMock.mock.calls[0] as [string, RequestInit])[1].body as string) as Record<string, unknown>
    expect(body).toMatchObject({ model: 'gpt-image-1', size: '1536x1024', n: 1, quality: 'medium', output_format: 'png' })
    expect(body).not.toHaveProperty('response_format')
  })

  it('speaks dall-e when that is the model', async () => {
    const fetchMock = vi.fn().mockResolvedValue(json({ data: [{ b64_json: Buffer.from(PNG).toString('base64') }] }))
    const images = new OpenAIImageGenerator({
      baseUrl: 'https://api.example/v1',
      apiKey: 'k',
      model: 'dall-e-3',
      fetch: fetchMock as unknown as typeof fetch,
    })
    await images.generate({ prompt: 'a tiny robot', aspect: 'portrait' })
    const body = JSON.parse((fetchMock.mock.calls[0] as [string, RequestInit])[1].body as string) as Record<string, unknown>
    expect(body).toMatchObject({ size: '1024x1792', response_format: 'b64_json' })
    expect(body).not.toHaveProperty('output_format')
  })

  it('downloads the image when the provider answers with a URL, and reports failures', async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(json({ data: [{ url: 'https://cdn.example/out.png' }] }))
      .mockResolvedValueOnce(new Response(PNG, { status: 200 }))
      .mockResolvedValueOnce(json({ error: { message: 'safety system' } }, 400))
    const images = new OpenAIImageGenerator({
      baseUrl: 'https://api.example/v1',
      apiKey: 'k',
      model: 'gpt-image-1',
      fetch: fetchMock as unknown as typeof fetch,
    })
    await expect(images.generate({ prompt: 'a tiny robot' })).resolves.toEqual({ data: PNG, mimeType: 'image/png' })
    await expect(images.generate({ prompt: 'a tiny robot' })).rejects.toThrow(/safety system/)
  })
})

describe('OpenAIVideoGenerator', () => {
  it('creates the job, polls until it is done, then downloads the clip', async () => {
    const mp4 = new Uint8Array([0, 0, 0, 0x18, 0x66, 0x74, 0x79, 0x70])
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(json({ id: 'video_1', status: 'queued' }))
      .mockResolvedValueOnce(json({ id: 'video_1', status: 'in_progress', progress: 40 }))
      .mockResolvedValueOnce(json({ id: 'video_1', status: 'completed' }))
      .mockResolvedValueOnce(new Response(mp4, { status: 200, headers: { 'content-type': 'video/mp4' } }))
    const sleep = vi.fn().mockResolvedValue(undefined)
    const videos = new OpenAIVideoGenerator({
      baseUrl: 'https://api.example/v1',
      apiKey: 'k',
      model: 'sora-2',
      seconds: 4,
      pollMs: 5,
      fetch: fetchMock as unknown as typeof fetch,
      sleep,
    })

    await expect(videos.generate({ prompt: 'a tiny robot waves', orientation: 'portrait' })).resolves.toEqual({
      data: mp4,
      mimeType: 'video/mp4',
      seconds: 4,
    })
    const calls = fetchMock.mock.calls as Array<[string, RequestInit]>
    expect(calls.map(([url]) => url)).toEqual([
      'https://api.example/v1/videos',
      'https://api.example/v1/videos/video_1',
      'https://api.example/v1/videos/video_1',
      'https://api.example/v1/videos/video_1/content',
    ])
    expect(JSON.parse(calls[0]![1].body as string)).toEqual({
      model: 'sora-2',
      prompt: 'a tiny robot waves',
      seconds: '4',
      size: '720x1280',
    })
    expect(sleep).toHaveBeenCalledTimes(2)
  })

  it('gives up on a failed render and on one that outlives the deadline', async () => {
    const failing = vi
      .fn()
      .mockResolvedValueOnce(json({ id: 'video_2', status: 'queued' }))
      .mockResolvedValueOnce(json({ id: 'video_2', status: 'failed', error: { message: 'moderation' } }))
    const videos = new OpenAIVideoGenerator({
      baseUrl: 'https://api.example/v1',
      apiKey: 'k',
      model: 'sora-2',
      fetch: failing as unknown as typeof fetch,
      sleep: vi.fn().mockResolvedValue(undefined),
    })
    await expect(videos.generate({ prompt: 'anything at all' })).rejects.toThrow(/moderation/)

    const slow = vi.fn().mockResolvedValue(json({ id: 'video_3', status: 'in_progress' }))
    const impatient = new OpenAIVideoGenerator({
      baseUrl: 'https://api.example/v1',
      apiKey: 'k',
      model: 'sora-2',
      timeoutMs: 0,
      fetch: slow as unknown as typeof fetch,
      sleep: vi.fn().mockResolvedValue(undefined),
    })
    await expect(impatient.generate({ prompt: 'anything at all' })).rejects.toThrow(/deadline/)
  })
})
