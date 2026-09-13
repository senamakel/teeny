import type { GeneratedImage, ImageAspect, ImageGenerator } from './types.js'
import { sniff } from './vision.js'

interface ImagesResponse {
  data?: Array<{ b64_json?: string; url?: string }>
  error?: { message?: string }
}

type Family = 'gpt-image' | 'dall-e' | 'other'

function familyOf(model: string): Family {
  if (model.startsWith('dall-e')) return 'dall-e'
  if (model.startsWith('gpt-image')) return 'gpt-image'
  return 'other'
}

/**
 * The size string for an aspect. Square is left to the endpoint's default --
 * the ladder fills in 1024x1024 and OpenAI's own models default to it -- so a
 * square request carries no size at all and can never name one a rung does
 * not know. The two OpenAI families disagree on their non-square sizes and
 * each rejects the other's; anything else gets the gpt-image spelling, which
 * is the one most resellers copy.
 */
function sizeFor(family: Family, aspect: ImageAspect): string | undefined {
  switch (aspect) {
    case 'landscape':
      return family === 'dall-e' ? '1792x1024' : '1536x1024'
    case 'portrait':
      return family === 'dall-e' ? '1024x1792' : '1024x1536'
    default:
      return undefined
  }
}

/**
 * Image generation through an OpenAI-shaped `/images/generations` endpoint:
 * the LLM ladder's `image` ladder, `gpt-image-*` or `dall-e-*` directly, or
 * anything else that answers that route. One image per call, always; the
 * tool that calls this is the place a second one would be rationed, not here.
 *
 * The bytes come back as base64 or as a URL, and as PNG, JPEG or WebP
 * depending on which rung served, so the type is read from the bytes rather
 * than assumed.
 */
export class OpenAIImageGenerator implements ImageGenerator {
  constructor(
    private readonly options: {
      baseUrl: string
      apiKey: string
      model: string
      /** `low`, `medium`, `high` for gpt-image; `standard`/`hd` for dall-e-3. Ignored elsewhere. */
      quality?: string
      timeoutMs?: number
      fetch?: typeof fetch
    },
  ) {}

  async generate(input: { prompt: string; aspect?: ImageAspect }): Promise<GeneratedImage> {
    const doFetch = this.options.fetch ?? fetch
    const { model } = this.options
    const family = familyOf(model)
    const size = sizeFor(family, input.aspect ?? 'square')
    const response = await doFetch(
      `${this.options.baseUrl.replace(/\/$/, '')}/images/generations`,
      {
        method: 'POST',
        headers: {
          authorization: `Bearer ${this.options.apiKey}`,
          'content-type': 'application/json',
        },
        body: JSON.stringify({
          model,
          prompt: input.prompt,
          n: 1,
          ...(size ? { size } : {}),
          // Only OpenAI's own families take these; on a ladder of many
          // models an unknown parameter is a 400 that does not advance it.
          ...(family !== 'other' && this.options.quality ? { quality: this.options.quality } : {}),
          ...(family === 'dall-e' ? { response_format: 'b64_json' } : {}),
          ...(family === 'gpt-image' ? { output_format: 'png' } : {}),
        }),
        signal: AbortSignal.timeout(this.options.timeoutMs ?? 180_000),
      },
    )
    const body = (await response.json().catch(() => ({}))) as ImagesResponse
    if (!response.ok) {
      throw new Error(`image generation failed (${response.status}): ${body.error?.message ?? 'no detail'}`)
    }
    const first = body.data?.[0]
    if (first?.b64_json) return typed(new Uint8Array(Buffer.from(first.b64_json, 'base64')))
    if (first?.url) {
      const download = await doFetch(first.url, { signal: AbortSignal.timeout(60_000) })
      if (!download.ok) throw new Error(`image download failed (${download.status})`)
      return typed(new Uint8Array(await download.arrayBuffer()))
    }
    throw new Error('image generation returned no image')
  }
}

function typed(data: Uint8Array): GeneratedImage {
  const sniffed = sniff(data)
  if (sniffed === 'image/png' || sniffed === 'image/jpeg' || sniffed === 'image/webp') {
    return { data, mimeType: sniffed }
  }
  throw new Error(`image generation returned ${sniffed ?? 'something that is not an image'}`)
}
