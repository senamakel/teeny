import { renderPersona, type Persona } from '../agent/soul.js'
import type { Config } from '../config.js'
import { OpenAITextModel } from '../media/reference.js'
import type { SafetyGate } from '../safety/gate.js'
import type { Memory, XTools } from '../types.js'
import { CoinGeckoLeaderboard } from './leaderboard.js'
import { MoonPipeline } from './pipeline.js'

/**
 * Builds the moon pipeline from configuration. Shared by the service and
 * the rehearsal script, so a dry run rehearses the pipeline that will post.
 */
export function buildMoonPipeline(
  config: Config,
  options: {
    memory: Memory
    x: XTools
    persona: Persona
    botUsername: string
    dryRun: boolean
    allowPosting?: boolean
    logger?: { info: (object: unknown, message?: string) => void }
    safety?: Pick<SafetyGate, 'outbound'>
  },
): MoonPipeline {
  return new MoonPipeline({
    source: new CoinGeckoLeaderboard(),
    text: new OpenAITextModel({
      baseUrl: config.LLM_BASE_URL,
      apiKey: config.LLM_API_KEY,
      model: config.LLM_MODEL,
    }),
    memory: options.memory,
    x: options.x,
    persona: renderPersona(options.persona),
    botUsername: options.botUsername,
    dryRun: options.dryRun,
    ...(options.allowPosting === false ? { allowPosting: false } : {}),
    coinId: config.MOON_COIN_ID,
    category: config.MOON_CATEGORY,
    boardName: config.MOON_BOARD_NAME,
    boardSize: config.MOON_BOARD_SIZE,
    rungs: config.MOON_RUNGS,
    minMultiple: config.MOON_MIN_MULTIPLE,
    maxMultiple: config.MOON_MAX_MULTIPLE,
    maxEmoji: config.MAX_EMOJI_PER_POST,
    outputDir: config.MOON_OUTPUT_DIR,
    watermark: config.CHART_WATERMARK ?? `@${options.botUsername}`,
    ...(options.logger ? { logger: options.logger } : {}),
    ...(options.safety ? { safety: options.safety } : {}),
  })
}
