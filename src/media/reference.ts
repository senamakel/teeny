import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, extname } from 'node:path'
import type { ImageInput, ImageMimeType, ImageReader } from './types.js'
import { sniff } from './vision.js'

/**
 * The account's own artwork, copied from tinyhumansworld.com into the repo
 * (see `assets/tiny/README.md`). These are what Tiny looks like; a generated
 * picture that ignores them is a picture of some other robot.
 */
export const DEFAULT_REFERENCE_IMAGES = [
  'assets/tiny/tiny-ai-narrativ.jpg',
  'assets/tiny/tiny-by-openhuman.jpg',
  'assets/tiny/tiny-and-friends.jpg',
  'assets/tiny/tiny-in-the-sun.jpg',
  'assets/tiny/asian-wagmi-tiny.jpg',
  'assets/tiny/github-tiny.jpg',
  'assets/tiny/tiny-community-work.jpg',
  'assets/tiny/tiny-the-phoenix-2.jpg',
  'assets/tiny/tiny-group-sand-3.jpg',
  'assets/tiny/tiny-love.jpg',
]

const BY_EXTENSION: Record<string, ImageMimeType> = {
  '.png': 'image/png',
  '.jpg': 'image/jpeg',
  '.jpeg': 'image/jpeg',
  '.webp': 'image/webp',
  '.gif': 'image/gif',
}

/** A reference is a URL or a path on disk; the reader takes either shape. */
export function referenceInput(reference: string): ImageInput {
  if (/^https?:\/\//i.test(reference)) return { url: reference }
  const data = new Uint8Array(readFileSync(reference))
  const mimeType = sniff(data) ?? BY_EXTENSION[extname(reference).toLowerCase()]
  if (!mimeType) throw new Error(`${reference} is not an image this can read`)
  return { data, mimeType }
}

const LOOK_QUESTION =
  'This is official artwork of a character called Tiny. Describe Tiny for an illustrator ' +
  'who has to draw the same character again without seeing this: body shape and ' +
  'proportions, head and face, eyes, colours with specific names, materials and texture, ' +
  'clothing or markings, anything always present. Then the art style: rendering, line, ' +
  'palette, lighting, mood. Say what is in the scene only in one short clause; the ' +
  'character and the style are what matter. Be concrete and dense; no preamble.'

const SHEET_INSTRUCTION =
  'Below are several descriptions of the same character, Tiny, each written from a ' +
  'different official picture. Write ONE character sheet for an image model, 120 to 180 ' +
  'words, that captures what is consistent across them: the body, the face, the colours ' +
  '(named precisely), the materials, the proportions, and the art style. Leave out ' +
  'anything that appears in only one picture, and leave out every scene. Write it as a ' +
  'plain paragraph starting "Tiny is", present tense, no headings, no bullets, nothing ' +
  'before or after the paragraph.'

interface CachedSheet {
  urls: string[]
  sheet: string
  looks: Record<string, string>
  createdAt: string
}

export interface TextModel {
  complete(input: { system?: string; prompt: string }): Promise<string>
}

/** A plain chat completion, for the one text step that distils the looks into a sheet. */
export class OpenAITextModel implements TextModel {
  constructor(
    private readonly options: { baseUrl: string; apiKey: string; model: string; fetch?: typeof fetch },
  ) {}

  async complete(input: { system?: string; prompt: string }): Promise<string> {
    const doFetch = this.options.fetch ?? fetch
    const response = await doFetch(`${this.options.baseUrl.replace(/\/$/, '')}/chat/completions`, {
      method: 'POST',
      headers: { authorization: `Bearer ${this.options.apiKey}`, 'content-type': 'application/json' },
      body: JSON.stringify({
        model: this.options.model,
        temperature: 0.2,
        messages: [
          ...(input.system ? [{ role: 'system', content: input.system }] : []),
          { role: 'user', content: input.prompt },
        ],
      }),
      signal: AbortSignal.timeout(90_000),
    })
    const body = (await response.json().catch(() => ({}))) as {
      choices?: Array<{ message?: { content?: string } }>
      error?: { message?: string }
    }
    if (!response.ok) {
      throw new Error(`text request failed (${response.status}): ${body.error?.message ?? 'no detail'}`)
    }
    const text = body.choices?.[0]?.message?.content?.trim()
    if (!text) throw new Error('the text model returned nothing')
    return text
  }
}

/**
 * What Tiny looks like, in words an image model can follow.
 *
 * The image ladder is text-to-image only -- reference-image input is a stated
 * non-goal there -- so the artwork cannot be handed to the generator as
 * pixels. It can be handed to the vision ladder, which describes each picture
 * for an illustrator; a text step then distils the descriptions into one
 * paragraph of what is constant across them. That paragraph is prepended to
 * every generation prompt that features Tiny.
 *
 * Built once and cached on disk, keyed by the URL list: vision calls cost
 * money and the artwork does not change between restarts. Every failure is a
 * missing sheet rather than a failed generation; the tool says so and the
 * prompt goes out without it.
 */
/** One of the reference pictures, and what the vision ladder saw in it. */
export interface ReferenceSeed {
  /** The path or URL the picture was read from. */
  url: string
  /** The illustrator's description of that one picture: pose, scene, palette, mood. */
  look: string
}

export class ReferenceSheet {
  private pending: Promise<string | undefined> | undefined
  private looks: Record<string, string> = {}

  constructor(
    private readonly options: {
      /** Paths on disk or https URLs. */
      urls: string[]
      reader: ImageReader
      text: TextModel
      cachePath?: string
      /** Reports how the build went; the default is silent. */
      log?: (event: string, detail?: Record<string, unknown>) => void
    },
  ) {}

  /** The sheet, built on first call and remembered; undefined when it could not be built. */
  get(): Promise<string | undefined> {
    this.pending ??= this.build().catch((error: unknown) => {
      this.options.log?.('reference sheet failed', {
        err: error instanceof Error ? error.message : String(error),
      })
      // Let the next caller try again rather than remembering a transient failure.
      this.pending = undefined
      return undefined
    })
    return this.pending
  }

  /** Starts the build without waiting for it, so the first generation does not pay for it. */
  warm(): void {
    void this.get()
  }

  /**
   * One reference picture, chosen at random, with its own description.
   *
   * The sheet is what is constant across the artwork; a seed is one picture
   * in particular -- the pose, the setting, the palette of that one -- and
   * a generation that starts from a seed comes out looking like a sibling of
   * a published picture rather than a generic rendering of the sheet. The
   * image ladder takes no picture as input, so the seed reaches it the same
   * way the sheet does: as words. Undefined until the sheet has been built,
   * or when no picture could be described.
   */
  async seed(random: () => number = Math.random): Promise<ReferenceSeed | undefined> {
    await this.get()
    const entries = Object.entries(this.looks).filter(([, look]) => look.trim())
    if (entries.length === 0) return undefined
    const [url, look] = entries[Math.min(entries.length - 1, Math.floor(random() * entries.length))]!
    return { url, look }
  }

  private async build(): Promise<string | undefined> {
    const urls = [...new Set(this.options.urls)]
    if (urls.length === 0) return undefined

    const cached = this.readCache()
    if (cached && sameUrls(cached.urls, urls)) {
      this.looks = cached.looks ?? {}
      this.options.log?.('reference sheet loaded from cache', { urls: urls.length })
      return cached.sheet
    }

    const looks: Record<string, string> = {}
    await Promise.all(
      urls.map(async (url) => {
        try {
          looks[url] = await this.options.reader.describe(referenceInput(url), LOOK_QUESTION)
        } catch (error) {
          this.options.log?.('could not look at a reference image', {
            url,
            err: error instanceof Error ? error.message : String(error),
          })
        }
      }),
    )
    const described = Object.values(looks)
    if (described.length === 0) throw new Error('none of the reference images could be described')
    this.looks = looks

    const sheet = await this.options.text.complete({
      system: 'You write terse, concrete visual briefs for image models.',
      prompt: `${SHEET_INSTRUCTION}\n\n${described.map((look, index) => `PICTURE ${index + 1}:\n${look}`).join('\n\n')}`,
    })
    const trimmed = sheet.replace(/\s+/g, ' ').trim()
    if (!/^tiny is/i.test(trimmed)) {
      this.options.log?.('reference sheet did not start with "Tiny is"; keeping it anyway')
    }
    this.writeCache({ urls, sheet: trimmed, looks, createdAt: new Date().toISOString() })
    this.options.log?.('reference sheet built', { pictures: described.length, words: trimmed.split(' ').length })
    return trimmed
  }

  private readCache(): CachedSheet | undefined {
    const path = this.options.cachePath
    if (!path) return undefined
    try {
      const parsed = JSON.parse(readFileSync(path, 'utf8')) as Partial<CachedSheet>
      if (Array.isArray(parsed.urls) && typeof parsed.sheet === 'string' && parsed.sheet.trim()) {
        return parsed as CachedSheet
      }
    } catch {
      // Absent or unreadable: build it again.
    }
    return undefined
  }

  private writeCache(sheet: CachedSheet): void {
    const path = this.options.cachePath
    if (!path) return
    try {
      mkdirSync(dirname(path), { recursive: true })
      writeFileSync(path, JSON.stringify(sheet, null, 2))
    } catch (error) {
      this.options.log?.('could not cache the reference sheet', {
        err: error instanceof Error ? error.message : String(error),
      })
    }
  }
}

function sameUrls(a: string[], b: string[]): boolean {
  if (a.length !== b.length) return false
  const sorted = [...a].sort()
  return [...b].sort().every((url, index) => url === sorted[index])
}

/** The generation prompt with Tiny's look in front of the scene. */
export function withReference(sheet: string | undefined, prompt: string): string {
  if (!sheet) return prompt
  return `${sheet}\n\nScene: ${prompt}`
}

/**
 * The generation prompt with the sheet and one seed picture in front of the
 * scene: what Tiny always looks like, then the particular picture today's
 * generation should look like a sibling of, then the scene.
 */
export function withSeed(
  sheet: string | undefined,
  seed: ReferenceSeed | undefined,
  prompt: string,
  options: { maxLength?: number } = {},
): string {
  if (!seed) return fit(withReference(sheet, prompt), options.maxLength)
  const parts = [
    ...(sheet ? [sheet] : []),
    `Style reference, one of Tiny's official pictures (match its rendering, palette and mood, not its scene): ${seed.look}`,
    `Scene: ${prompt}`,
  ]
  const composed = parts.join('\n\n')
  // Over the limit, the seed is the part to lose: the sheet is what makes it
  // Tiny and the scene is what makes it today.
  if (options.maxLength && composed.length > options.maxLength) {
    return fit(withReference(sheet, prompt), options.maxLength)
  }
  return composed
}

/** Under the limit, cut at a sentence when one is near enough. */
function fit(prompt: string, maxLength: number | undefined): string {
  if (!maxLength || prompt.length <= maxLength) return prompt
  const head = prompt.slice(0, maxLength)
  const stop = head.lastIndexOf('. ')
  return stop > maxLength * 0.7 ? head.slice(0, stop + 1) : head
}
