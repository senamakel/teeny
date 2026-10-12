import type { Gif, GifSource, XMedia } from '../types.js'

interface GiphyImage {
  url?: string
  mp4?: string
  size?: string
}

interface GiphyGif {
  id: string
  title?: string
  url?: string
  username?: string
  images?: Record<string, GiphyImage>
}

/** X rejects an animated GIF over 15 MB, so the largest usable rendition wins. */
const MAX_UPLOAD_BYTES = 15 * 1024 * 1024

const RENDITIONS = ['downsized_medium', 'downsized', 'original', 'fixed_height'] as const

function pickRendition(gif: GiphyGif): { url: string; mimeType: 'image/gif' } | undefined {
  for (const name of RENDITIONS) {
    const image = gif.images?.[name]
    const size = Number(image?.size ?? '0')
    if (image?.url && size > 0 && size <= MAX_UPLOAD_BYTES) {
      return { url: image.url, mimeType: 'image/gif' }
    }
  }
  return undefined
}

function toGif(gif: GiphyGif): Gif | undefined {
  const rendition = pickRendition(gif)
  if (!rendition || !gif.id) return undefined
  return {
    id: gif.id,
    title: gif.title?.trim() || '(untitled)',
    pageUrl: gif.url ?? `https://giphy.com/gifs/${gif.id}`,
    mediaUrl: rendition.url,
    mimeType: rendition.mimeType,
  }
}

/**
 * Giphy search, scoped to one channel by default.
 *
 * Giphy has no by-username endpoint in the public v1 API; a channel is searched
 * by putting `@username` in the query, which is what the website does too.
 */
export class GiphyClient implements GifSource {
  constructor(
    private readonly options: { apiKey: string; channel: string; limit?: number },
  ) {}

  async search(query: string, options: { channelOnly?: boolean } = {}): Promise<Gif[]> {
    const channel = this.options.channel.replace(/^@/, '')
    const scoped = options.channelOnly === false ? query : `@${channel} ${query}`.trim()
    const url = new URL('https://api.giphy.com/v1/gifs/search')
    url.searchParams.set('api_key', this.options.apiKey)
    url.searchParams.set('q', scoped)
    url.searchParams.set('limit', String(this.options.limit ?? 10))
    url.searchParams.set('rating', 'g')
    url.searchParams.set('bundle', 'messaging_non_clips')

    const response = await fetch(url, { signal: AbortSignal.timeout(15_000) })
    if (!response.ok) {
      throw new Error(`Giphy search failed (${response.status}): ${await response.text()}`)
    }
    const body = (await response.json()) as { data?: GiphyGif[] }

    return (body.data ?? []).flatMap((gif) => {
      const converted = toGif(gif)
      return converted ? [converted] : []
    })
  }

  async get(id: string): Promise<Gif | undefined> {
    const url = new URL(`https://api.giphy.com/v1/gifs/${encodeURIComponent(id)}`)
    url.searchParams.set('api_key', this.options.apiKey)

    const response = await fetch(url, { signal: AbortSignal.timeout(15_000) })
    if (response.status === 404) return undefined
    if (!response.ok) {
      throw new Error(`Giphy lookup failed (${response.status}): ${await response.text()}`)
    }
    const body = (await response.json()) as { data?: GiphyGif }
    return body.data ? toGif(body.data) : undefined
  }

  async download(gif: Gif): Promise<XMedia> {
    const response = await fetch(gif.mediaUrl, { signal: AbortSignal.timeout(30_000) })
    if (!response.ok) {
      throw new Error(`Giphy download failed (${response.status}) for ${gif.mediaUrl}`)
    }
    const data = new Uint8Array(await response.arrayBuffer())
    if (data.byteLength > MAX_UPLOAD_BYTES) {
      throw new Error(`GIF ${gif.id} is ${data.byteLength} bytes, over the X upload limit`)
    }
    return { data, mimeType: gif.mimeType }
  }
}
