/**
 * Media the account can look at and media it can make.
 *
 * Three capabilities, three interfaces, so each can be backed by whatever
 * provider answers and switched off independently: a text-only model on the
 * ladder means no vision, and an image key without a video key means the
 * account can draw but not film. Nothing here publishes; the bytes go through
 * the same `uploadMedia` and `sendPhoto` paths as a GIF or a chart.
 */

export type ImageMimeType = 'image/png' | 'image/jpeg' | 'image/webp' | 'image/gif'

/** An image to look at: either bytes this service already holds, or a URL. */
export type ImageInput =
  | { data: Uint8Array; mimeType: ImageMimeType }
  | { url: string }

export interface ImageReader {
  /**
   * What is in the image, in words, answering `question` when one is asked.
   * The answer is written by a model looking at a picture a stranger sent, so
   * callers treat it as untrusted text like any other tool result.
   */
  describe(image: ImageInput, question?: string): Promise<string>
}

export type ImageAspect = 'square' | 'landscape' | 'portrait'

export interface GeneratedImage {
  data: Uint8Array
  mimeType: 'image/png' | 'image/jpeg' | 'image/webp'
}

export interface ImageGenerator {
  generate(input: { prompt: string; aspect?: ImageAspect }): Promise<GeneratedImage>
}

export type VideoOrientation = 'landscape' | 'portrait'

export interface GeneratedVideo {
  data: Uint8Array
  mimeType: 'video/mp4'
  seconds?: number
}

export interface VideoGenerator {
  generate(input: { prompt: string; orientation?: VideoOrientation }): Promise<GeneratedVideo>
}

/**
 * A generated file, held until a publisher attaches it. `id` is what the
 * drafter passes back; the bytes never go through the model.
 */
export interface StoredMedia {
  id: string
  kind: 'image' | 'video'
  data: Uint8Array
  mimeType: GeneratedImage['mimeType'] | 'video/mp4'
  prompt: string
  createdAt: number
  /** Where a copy was written for inspection, when the store has a directory. */
  path?: string
}
