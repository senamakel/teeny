import { createExaClient } from '../agent/tools/exa.js'
import { renderPersona, type Persona } from '../agent/soul.js'
import type { Config } from '../config.js'
import { OpenAITextModel } from '../media/reference.js'
import type { MediaAccess } from '../media/wiring.js'
import type { SafetyGate } from '../safety/gate.js'
import type { Critic } from '../critic/algo.js'
import type { InferenceLedger } from '../ledger/ledger.js'
import { buildBackrooms } from '../backrooms/wiring.js'
import type { Memory, XTools, XTrend } from '../types.js'
import { ExaNewsSource } from './news.js'
import { MorningPipeline } from './pipeline.js'
import { XTimelineSource, parseTimelineQueries } from './timeline.js'

/**
 * Builds the morning pipeline from configuration.
 *
 * Shared by the service and the rehearsal script, as the chart agent's
 * builder is, so a dry run rehearses the pipeline that will actually post.
 * It needs three things the configuration may not have: a news key, an
 * image generator, and the media access those live in. Missing any, it says
 * which, because a morning post without a picture is not the feature.
 */
export function buildMorningPipeline(
  config: Config,
  options: {
    memory: Memory
    /** The X client; when it can read trends, the morning reads them too. */
    x: XTools & { trends?(options?: { woeid?: number }): Promise<XTrend[]> }
    media: MediaAccess | undefined
    persona: Persona
    botUsername: string
    dryRun: boolean
    allowPosting?: boolean
    logger?: { info: (object: unknown, message?: string) => void }
    /** The content filter; the pipeline reads it on its prompts and its caption. */
    safety?: Pick<SafetyGate, 'outbound'>
    /** The ranker's reader; the pipeline shows it the script and the caption. */
    critic?: Critic
    /** Where every text step is written down. */
    ledger?: InferenceLedger
  },
): MorningPipeline {
  if (!config.EXA_API_KEY) throw new Error('EXA_API_KEY is not set, so there is no news to read')
  if (!options.media?.images) {
    throw new Error('IMAGE_GEN_ENABLED is off, so there is nothing to draw the morning post with')
  }
  const exa = createExaClient(config.EXA_API_KEY)
  const backrooms = config.MORNING_BACKROOMS
    ? buildBackrooms(config, {
        memory: options.memory,
        x: { searchPosts: (query, searchOptions) => options.x.searchPosts(query, searchOptions) },
        exa,
        botUsername: options.botUsername,
        persona: renderPersona(options.persona),
        maxEmoji: config.MAX_EMOJI_PER_POST,
        ...(options.ledger ? { ledger: options.ledger } : {}),
        ...(options.logger ? { logger: options.logger } : {}),
      })
    : undefined
  return new MorningPipeline({
    news: new ExaNewsSource(exa),
    ...(backrooms ? { backrooms, backroomsTurns: config.MORNING_BACKROOMS_TURNS } : {}),
    ...(config.MORNING_X_POSTS_PER_QUERY > 0
      ? {
          timeline: new XTimelineSource({
            searchPosts: (query, searchOptions) => options.x.searchPosts(query, searchOptions),
            ...(options.x.trends && config.MORNING_X_TRENDS_WOEID > 0
              ? { trends: () => options.x.trends!({ woeid: config.MORNING_X_TRENDS_WOEID }) }
              : {}),
          }),
          timelineQueries: parseTimelineQueries(config.MORNING_X_QUERIES),
          timelineHours: config.MORNING_X_HOURS,
          postsPerQuery: config.MORNING_X_POSTS_PER_QUERY,
        }
      : {}),
    text: new OpenAITextModel({
      baseUrl: config.LLM_BASE_URL,
      apiKey: config.LLM_API_KEY,
      model: config.LLM_MODEL,
      source: 'morning',
      ...(options.ledger ? { ledger: options.ledger } : {}),
    }),
    images: options.media.images,
    ...(options.media.videos ? { videos: options.media.videos } : {}),
    ...(options.media.reader ? { reader: options.media.reader } : {}),
    ...(options.media.reference ? { reference: options.media.reference } : {}),
    store: options.media.store,
    budget: options.media.budget,
    memory: options.memory,
    x: options.x,
    persona: renderPersona(options.persona),
    botUsername: options.botUsername,
    dryRun: options.dryRun,
    ...(options.allowPosting === false ? { allowPosting: false } : {}),
    topics: config.MORNING_NEWS_TOPICS,
    newsDays: config.MORNING_NEWS_DAYS,
    storiesPerTopic: config.MORNING_STORIES_PER_TOPIC,
    videoSeconds: config.MORNING_VIDEO_SECONDS,
    maxEmoji: config.MAX_EMOJI_PER_POST,
    ...(options.logger ? { logger: options.logger } : {}),
    ...(options.safety ? { safety: options.safety } : {}),
    ...(options.critic ? { critic: options.critic } : {}),
  })
}
