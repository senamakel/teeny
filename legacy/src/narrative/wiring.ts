import type { TelegramApi } from '../clients/telegram.js'
import type { Config } from '../config.js'
import type { Critic } from '../critic/algo.js'
import type { InferenceLedger } from '../ledger/ledger.js'
import { OpenAITextModel } from '../media/reference.js'
import type { MediaAccess } from '../media/wiring.js'
import type { SafetyGate } from '../safety/gate.js'
import type { Memory, XTools } from '../types.js'
import { NarrativePipeline } from './pipeline.js'
import type { Episode } from './stages.js'
import { FileNarrativeState } from './state.js'
import { loadNarrativeVoice } from './voice.js'
import { buildWorldState } from './world-state.js'
import type { ExaSearch } from '../agent/tools/exa.js'
import { LikenessFinder } from './likeness.js'

/**
 * Builds the serial's pipeline from configuration. Shared by the service
 * and the rehearsal script, so a dry run rehearses the pipeline that posts.
 */
export function buildNarrativePipeline(
  config: Config,
  options: {
    memory: Memory
    x: XTools & { identity?(): Promise<{ id: string; username: string; followers?: number }> }
    media: MediaAccess | undefined
    /** The web, for a visitor's photograph. Absent, visitors are drawn from their description alone. */
    exa?: ExaSearch
    telegram?: Pick<TelegramApi, 'sendVideo' | 'sendPhoto'>
    botUsername: string
    dryRun: boolean
    allowPosting?: boolean
    logger?: { info: (object: unknown, message?: string) => void }
    safety?: Pick<SafetyGate, 'outbound'>
    critic?: Critic
    ledger?: InferenceLedger
    /** Overrides for the rehearsal script. */
    branches?: number
    turns?: number
    roomOnly?: boolean
    cut?: Episode
    visitor?: string
  },
): NarrativePipeline {
  if (!options.media?.images) {
    throw new Error('IMAGE_GEN_ENABLED is off, so there is nothing to draw the serial with')
  }
  const model = (source: string, temperature?: number): OpenAITextModel =>
    new OpenAITextModel({
      baseUrl: config.LLM_BASE_URL,
      apiKey: config.LLM_API_KEY,
      model: config.LLM_MODEL,
      source,
      ...(temperature === undefined ? {} : { temperature }),
      ...(options.ledger ? { ledger: options.ledger } : {}),
    })
  const chatIds = config.NARRATIVE_TELEGRAM_CHAT_IDS.map(Number).filter((id) => Number.isFinite(id))
  return new NarrativePipeline({
    voice: loadNarrativeVoice({ voicePath: config.NARRATIVE_VOICE_PATH, lorePath: config.NARRATIVE_LORE_PATH }),
    state: new FileNarrativeState(config.NARRATIVE_STATE_PATH),
    worldState: (previous) =>
      buildWorldState(
        {
          ...(options.x.identity ? { followers: () => options.x.identity!().then((who) => who.followers) } : {}),
          mentions: () =>
            options.x.searchPosts(`(@${options.botUsername} OR teeny OR $${config.CHART_TOKEN_SYMBOL}) -is:retweet -from:${options.botUsername}`, {
              maxResults: 50,
              startTime: new Date(Date.now() - 24 * 60 * 60 * 1_000).toISOString(),
            }),
          ...(previous.lastEpisodeId
            ? {
                lastEpisodeMetrics: () =>
                  options.x.postMetrics([previous.lastEpisodeId!]).then((rows) => {
                    const row = rows[0]
                    return row ? { likes: row.likes, replies: row.replies } : undefined
                  }),
              }
            : {}),
        },
        {
          episode: previous.episode + 1,
          ...(previous.episode > 0
            ? {
                lastEpisode: {
                  number: previous.episode,
                  ...(previous.lastTitle ? { title: previous.lastTitle } : {}),
                  ...(previous.stateAfter ? { stateAfter: previous.stateAfter } : {}),
                },
              }
            : {}),
        },
      ),
    wild: model('narrative:teeny', config.NARRATIVE_WILD_TEMPERATURE),
    straight: model('narrative:producer', config.NARRATIVE_STRAIGHT_TEMPERATURE),
    text: model('narrative'),
    images: options.media.images,
    ...(options.media.videos ? { videos: options.media.videos } : {}),
    ...(options.media.reader ? { reader: options.media.reader } : {}),
    ...(options.media.reference ? { reference: options.media.reference } : {}),
    store: options.media.store,
    budget: options.media.budget,
    memory: options.memory,
    x: options.x,
    ...(options.telegram && chatIds.length > 0 ? { telegram: { api: options.telegram, chatIds } } : {}),
    ...(options.exa && options.media.reader
      ? { likeness: new LikenessFinder({ exa: options.exa, reader: options.media.reader, cacheDir: config.NARRATIVE_LIKENESS_DIR }) }
      : {}),
    ...(options.critic ? { critic: options.critic } : {}),
    ...(options.safety ? { safety: options.safety } : {}),
    botUsername: options.botUsername,
    branches: options.branches ?? config.NARRATIVE_BRANCHES,
    turns: options.turns ?? config.NARRATIVE_TURNS,
    videoSeconds: config.NARRATIVE_VIDEO_SECONDS,
    dryRun: options.dryRun,
    ...(options.allowPosting === false ? { allowPosting: false } : {}),
    ...(options.roomOnly ? { roomOnly: true } : {}),
    ...(options.cut ? { cut: options.cut } : {}),
    ...(options.visitor ? { visitor: options.visitor } : {}),
    ...(options.logger ? { logger: options.logger } : {}),
  })
}
