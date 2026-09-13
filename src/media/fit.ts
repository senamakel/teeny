import sharp from 'sharp'
import type { XMedia } from '../types.js'

/** X refuses a still image over this; GIFs and videos have their own, larger limits. */
export const X_IMAGE_MAX_BYTES = 5 * 1024 * 1024

/**
 * A still image small enough for X.
 *
 * The image ladder's rungs render what they like: one served a 2048-square
 * PNG of nine megabytes, which X's `tweet_image` pipeline rejects outright.
 * Anything over the limit is re-encoded as JPEG at a high quality, and
 * scaled down a step at a time if that is still not enough. GIFs and videos
 * pass through untouched; so does anything already under the limit, so the
 * common case costs nothing.
 */
export async function fitImageForX(media: XMedia): Promise<XMedia> {
  if (!media.mimeType.startsWith('image/') || media.mimeType === 'image/gif') return media
  if (media.data.byteLength <= X_IMAGE_MAX_BYTES) return media

  let width: number | undefined
  for (let quality = 90; ; quality -= 10) {
    let image = sharp(media.data)
    if (width) image = image.resize({ width, withoutEnlargement: true })
    const data = new Uint8Array(await image.jpeg({ quality, mozjpeg: true }).toBuffer())
    if (data.byteLength <= X_IMAGE_MAX_BYTES) return { data, mimeType: 'image/jpeg' }
    if (quality <= 70) {
      // Quality has been spent; halve the side and start again.
      const meta = await sharp(media.data).metadata()
      width = Math.floor((width ?? meta.width ?? 2048) / 2)
      if (width < 256) throw new Error('could not fit the image under X\'s size limit')
      quality = 100
    }
  }
}
