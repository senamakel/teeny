import type { Config } from '../config.js'
import { OpenAIImageGenerator } from './images.js'
import { MediaBudget, MediaStore } from './store.js'
import type { ImageGenerator, ImageReader, VideoGenerator } from './types.js'
import { OpenAIVideoGenerator } from './videos.js'
import { OpenAIVisionReader } from './vision.js'

/**
 * Everything media-shaped a turn may reach, built once and shared by every
 * drafter in the process so the budget is one budget and a generated id can
 * be attached by whichever job asked for it.
 */
export interface MediaAccess {
  store: MediaStore
  budget: MediaBudget
  /** Absent: the account cannot see pictures and the prompts say so. */
  reader?: ImageReader
  images?: ImageGenerator
  videos?: VideoGenerator
}

/**
 * Builds media access from configuration, or nothing at all when no part of
 * it is configured, so the tool list and the prompts stay exactly as they
 * were on a box that has none of these keys.
 */
export function buildMedia(config: Config): MediaAccess | undefined {
  const reader = config.VISION_MODEL
    ? new OpenAIVisionReader({
        baseUrl: config.VISION_BASE_URL ?? config.LLM_BASE_URL,
        apiKey: config.VISION_API_KEY ?? config.LLM_API_KEY,
        model: config.VISION_MODEL,
      })
    : undefined
  const images = config.IMAGE_GEN_API_KEY
    ? new OpenAIImageGenerator({
        baseUrl: config.IMAGE_GEN_BASE_URL,
        apiKey: config.IMAGE_GEN_API_KEY,
        model: config.IMAGE_GEN_MODEL,
        quality: config.IMAGE_GEN_QUALITY,
      })
    : undefined
  const videos = config.VIDEO_GEN_API_KEY
    ? new OpenAIVideoGenerator({
        baseUrl: config.VIDEO_GEN_BASE_URL,
        apiKey: config.VIDEO_GEN_API_KEY,
        model: config.VIDEO_GEN_MODEL,
        seconds: config.VIDEO_GEN_SECONDS,
        timeoutMs: config.VIDEO_GEN_TIMEOUT_S * 1_000,
      })
    : undefined
  if (!reader && !images && !videos) return undefined
  return {
    store: new MediaStore({ outputDir: config.MEDIA_OUTPUT_DIR }),
    budget: new MediaBudget({
      imagesPerDay: config.MEDIA_IMAGES_PER_DAY,
      videosPerDay: config.MEDIA_VIDEOS_PER_DAY,
    }),
    ...(reader ? { reader } : {}),
    ...(images ? { images } : {}),
    ...(videos ? { videos } : {}),
  }
}
