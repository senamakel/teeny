import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import type { ExaSearch } from '../agent/tools/exa.js'
import type { ImageReader } from '../media/types.js'
import { isFetchableImageUrl } from '../media/vision.js'

/**
 * What a visitor actually looks like, for a generator that only takes words.
 *
 * The image and video rungs accept a prompt and nothing else, so a
 * photograph cannot be handed to them; what can is a description written
 * by a model that has looked at one. The web is searched for a recent
 * photograph of the person, the vision rung is asked to describe them as a
 * casting note -- face, hair, build, what they wear, how they hold
 * themselves -- and that note goes into the prompt next to the name. A
 * likeness is written once per person and kept, because the person does
 * not change between episodes and the search costs a call.
 */
export interface Likeness {
  name: string
  description: string
  photoUrl: string
  writtenAt: string
}

export interface LikenessSource {
  find(name: string): Promise<Likeness | undefined>
}

const QUESTION = [
  'Describe the person in this photograph in 40 to 70 words so an illustrator could draw them without',
  'seeing it: face shape, hair (colour, length, how it sits), eyes and brows, skin tone, build and',
  'height impression, the clothes they are wearing here, and one thing about how they hold themselves.',
  'If more than one person is in the photo, describe only the one most prominently in the centre.',
  'Plain descriptive words, present tense, no name, no opinions. Answer with the description only.',
].join(' ')

export class LikenessFinder implements LikenessSource {
  constructor(
    private readonly options: {
      exa: Pick<ExaSearch, 'searchAndContents'>
      reader: ImageReader
      cacheDir?: string
      logger?: { info: (object: unknown, message?: string) => void }
    },
  ) {}

  async find(name: string): Promise<Likeness | undefined> {
    const cached = await this.read(name)
    if (cached) return cached
    const photoUrl = await this.photo(name)
    if (!photoUrl) return undefined
    const description = (await this.options.reader.describe({ url: photoUrl }, QUESTION)).replace(/\s+/g, ' ').trim()
    if (description.length < 20) return undefined
    const likeness: Likeness = { name, description: description.slice(0, 600), photoUrl, writtenAt: new Date().toISOString() }
    await this.write(likeness)
    this.options.logger?.info({ name, photoUrl }, 'narrative: wrote a likeness')
    return likeness
  }

  /**
   * A picture of the person, from a search that asks for one. A picture
   * whose own URL names the person is taken first, because a page about
   * them often leads with a group shot or a product; then any picture on a
   * page whose title names them.
   */
  private async photo(name: string): Promise<string | undefined> {
    const { results } = await this.options.exa.searchAndContents(`${name} photo portrait`, {
      numResults: 8,
      type: 'auto',
      contents: { extras: { imageLinks: 3 } },
    })
    const hits = results as Array<{ url: string; title?: string | null; image?: string; extras?: { imageLinks?: string[] } }>
    const slug = name.trim().toLowerCase().split(/\s+/)
    const names = (text: string | null | undefined): boolean => slug.every((part) => (text ?? '').toLowerCase().includes(part))
    const candidates = hits.flatMap((hit) =>
      [hit.image, ...(hit.extras?.imageLinks ?? [])]
        .map((raw) => (raw ?? '').replace(/&amp;/g, '&'))
        .filter((url) => url && isFetchableImageUrl(url) && /\.(jpe?g|png|webp)(\?|$)/i.test(url))
        .map((url) => ({ url, page: hit })),
    )
    return (
      candidates.find(({ url }) => names(url.replace(/[-_/.]+/g, ' ')))?.url ??
      candidates.find(({ page }) => names(page.title) || names(page.url.replace(/[-_/.]+/g, ' ')))?.url ??
      candidates[0]?.url
    )
  }

  private path(name: string): string | undefined {
    if (!this.options.cacheDir) return undefined
    return join(this.options.cacheDir, `${name.toLowerCase().replace(/[^a-z0-9]+/g, '-')}.json`)
  }

  private async read(name: string): Promise<Likeness | undefined> {
    const path = this.path(name)
    if (!path) return undefined
    try {
      return JSON.parse(await readFile(path, 'utf8')) as Likeness
    } catch {
      return undefined
    }
  }

  private async write(likeness: Likeness): Promise<void> {
    const path = this.path(likeness.name)
    if (!path) return
    await mkdir(this.options.cacheDir!, { recursive: true })
    await writeFile(path, `${JSON.stringify(likeness, null, 2)}\n`)
  }
}

/** The visitor's name in a still or shot list, followed once by what they look like. */
export function withLikeness(text: string, visitor: { name: string }, likeness: Likeness): string {
  const pattern = new RegExp(visitor.name.trim().replace(/[.*+?^${}()|[\]\\]/g, '\\$&'), 'i')
  if (!pattern.test(text)) return `${visitor.name} (${likeness.description}) is in the scene. ${text}`
  return text.replace(pattern, (match) => `${match} (${likeness.description})`)
}
