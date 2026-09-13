import type { GeneratedImage, ImageAspect, ImageGenerator } from './types.js'

interface ImagesResponse {
  data?: Array<{ b64_json?: string; url?: string }>
  error?: { message?: string }
}

/**
 * The size string each model family wants for an aspect. The two families
 * disagree on their non-square sizes and each rejects the other's, so the
 * caller asks for a shape and the model's own vocabulary is used here.
 */
function sizeFor(model: string, aspect: ImageAspect): string {
  const dalle = model.startsWith('dall-e')
  switch (aspect) {
    case 'landscape':
      return dalle ? '1792x1024' : '1536x1024'
    case 'portrait':
      return dalle ? '1024x1792' : '1024x1536'
    default:
      return '1024x1024'
  }
}

/**
 * Image generation through an OpenAI-shaped `/images/generations` endpoint:
 * `gpt-image-*`, `dall-e-*`, or anything else that answers that route with
 * base64 PNG bytes. One image per call, always; the tool that calls this is
 * the place a second one would be rationed, not here.
 */
export class OpenAIImageGenerator implements ImageGenerator {
  constructor(
    private readonly options: {
      baseUrl: string
      apiKey: string
      model: string
      /** `low`, `medium`, `high` for gpt-image; `standard`/`hd` for dall-e-3. */
      quality?: string
      timeoutMs?: number
      fetch?: typeof fetch
    },
  ) {}

  async generate(input: { prompt: string; aspect?: ImageAspect }): Promise<GeneratedImage> {
    const doFetch = this.options.fetch ?? fetch
    const { model } = this.options
    const dalle = model.startsWith('dall-e')
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
          size: sizeFor(model, input.aspect ?? 'square'),
          ...(this.options.quality ? { quality: this.options.quality } : {}),
          // gpt-image returns base64 and rejects the parameter; dall-e defaults
          // to a URL that expires in an hour and has to be asked for bytes.
          ...(dalle ? { response_format: 'b64_json' } : { output_format: 'png' }),
        }),
        signal: AbortSignal.timeout(this.options.timeoutMs ?? 180_000),
      },
    )
    const body = (await response.json().catch(() => ({}))) as ImagesResponse
    if (!response.ok) {
      throw new Error(`image generation failed (${response.status}): ${body.error?.message ?? 'no detail'}`)
    }
    const first = body.data?.[0]
    if (first?.b64_json) {
      return { data: new Uint8Array(Buffer.from(first.b64_json, 'base64')), mimeType: 'image/png' }
    }
    if (first?.url) {
      const download = await doFetch(first.url, { signal: AbortSignal.timeout(60_000) })
      if (!download.ok) throw new Error(`image download failed (${download.status})`)
      return { data: new Uint8Array(await download.arrayBuffer()), mimeType: 'image/png' }
    }
    throw new Error('image generation returned no image')
  }
}
