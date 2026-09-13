import type { Config } from '../config.js'
import { OpenAIImageGenerator } from './images.js'
import { OpenAITextModel, ReferenceSheet } from './reference.js'
import { MediaBudget, MediaStore } from './store.js'
import type { ImageGenerator, ImageReader, VideoGenerator } from './types.js'
import { LadderVideoGenerator, OpenAIVideoGenerator } from './videos.js'
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
  /** What Tiny looks like, for the generators. Absent without a reader or with the list off. */
  reference?: ReferenceSheet
}

/**
 * Builds media access from configuration, or nothing at all when every part
 * is off, so the tool list and the prompts stay exactly as they were on a
 * box that has none of it.
 *
 * Every endpoint defaults to the LLM's: the ladder serves vision, images and
 * video on the same URL and key as chat, so the common case is one setting.
 */
export function buildMedia(
  config: Config,
  options: { log?: (event: string, detail?: Record<string, unknown>) => void } = {},
): MediaAccess | undefined {
  const visionModel = config.VISION_MODEL.trim()
  const reader = visionModel
    ? new OpenAIVisionReader({
        baseUrl: config.VISION_BASE_URL ?? config.LLM_BASE_URL,
        apiKey: config.VISION_API_KEY ?? config.LLM_API_KEY,
        model: visionModel,
      })
    : undefined
  const images = config.IMAGE_GEN_ENABLED
    ? new OpenAIImageGenerator({
        baseUrl: config.IMAGE_GEN_BASE_URL ?? config.LLM_BASE_URL,
        apiKey: config.IMAGE_GEN_API_KEY ?? config.LLM_API_KEY,
        model: config.IMAGE_GEN_MODEL,
        ...(config.IMAGE_GEN_QUALITY ? { quality: config.IMAGE_GEN_QUALITY } : {}),
      })
    : undefined
  const videoOptions = {
    baseUrl: config.VIDEO_GEN_BASE_URL ?? config.LLM_BASE_URL,
    apiKey: config.VIDEO_GEN_API_KEY ?? config.LLM_API_KEY,
    model: config.VIDEO_GEN_MODEL,
    timeoutMs: config.VIDEO_GEN_TIMEOUT_S * 1_000,
  }
  const videos = !config.VIDEO_GEN_ENABLED
    ? undefined
    : config.VIDEO_GEN_API === 'openai'
      ? new OpenAIVideoGenerator({ ...videoOptions, seconds: config.VIDEO_GEN_SECONDS })
      : new LadderVideoGenerator(videoOptions)
  if (!reader && !images && !videos) return undefined

  // The sheet needs eyes to be built and a generator to be worth building.
  const reference =
    reader && (images || videos) && config.TINY_REFERENCE_IMAGES.length > 0
      ? new ReferenceSheet({
          urls: config.TINY_REFERENCE_IMAGES,
          reader,
          text: new OpenAITextModel({
            baseUrl: config.LLM_BASE_URL,
            apiKey: config.LLM_API_KEY,
            model: config.LLM_MODEL,
          }),
          cachePath: config.REFERENCE_SHEET_PATH,
          ...(options.log ? { log: options.log } : {}),
        })
      : undefined

  return {
    store: new MediaStore({ outputDir: config.MEDIA_OUTPUT_DIR }),
    budget: new MediaBudget({
      imagesPerDay: config.MEDIA_IMAGES_PER_DAY,
      videosPerDay: config.MEDIA_VIDEOS_PER_DAY,
    }),
    ...(reader ? { reader } : {}),
    ...(images ? { images } : {}),
    ...(videos ? { videos } : {}),
    ...(reference ? { reference } : {}),
  }
}
