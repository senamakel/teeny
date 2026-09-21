import { readFileSync } from 'node:fs'
import type { ExaSearch } from '../agent/tools/exa.js'
import type { Thought } from '../backrooms/distill.js'
import type { Craziness } from '../backrooms/evaluate.js'
import type { BackroomsRun } from '../backrooms/types.js'
import { buildBackrooms, seedFromForum, seedFromNews, type Backrooms } from '../backrooms/wiring.js'
import type { Config } from '../config.js'
import type { Critic } from '../critic/algo.js'
import type { InferenceLedger } from '../ledger/ledger.js'
import { OpenAITextModel } from '../media/reference.js'
import { MediaBudget, MediaStore } from '../media/store.js'
import type { MediaAccess } from '../media/wiring.js'
import { FileNarrativeState } from '../narrative/state.js'
import { parseLore } from '../narrative/voice.js'
import type { SafetyGate } from '../safety/gate.js'
import type { Memory, XPost, XTools } from '../types.js'
import { ArticlePipeline } from './pipeline.js'
import { loadHumanizer } from './proofread.js'
import { FileArticleStore, type ArticleStore } from './store.js'

/**
 * Builds the article pipeline from configuration. Shared by the service
 * and the rehearsal script, so a dry run rehearses the pipeline that
 * publishes. Unlike the serial it does not insist on an image generator:
 * an article without pictures is still an article, and the run says so.
 */
export function buildArticlePipeline(
  config: Config,
  options: {
    memory: Memory
    x: Pick<XTools, 'uploadMedia' | 'publishArticle'> & { searchPosts(query: string, options: { maxResults: number }): Promise<XPost[]> }
    media: MediaAccess | undefined
    exa?: ExaSearch
    botUsername: string
    /** Whether anything goes to X. The service passes ARTICLE_PUBLISH and not DRY_RUN; the script, its --publish flag. */
    publish: boolean
    logger?: { info: (object: unknown, message?: string) => void }
    safety?: Pick<SafetyGate, 'outbound'>
    critic?: Critic
    ledger?: InferenceLedger
    articles?: ArticleStore
    /** Overrides for the rehearsal script. */
    backrooms?: Pick<Backrooms, 'run'>
    rooms?: number
    structureOnly?: boolean
    noImages?: boolean
    noProof?: boolean
    noFlow?: boolean
    from?: { run: BackroomsRun; thought: Thought; judge?: Craziness; path?: string }
    topic?: string
    engine?: string
    seedText?: string
    brief?: string
  },
): ArticlePipeline {
  const model = (source: string, temperature: number): OpenAITextModel =>
    new OpenAITextModel({
      baseUrl: config.LLM_BASE_URL,
      apiKey: config.LLM_API_KEY,
      model: config.LLM_MODEL,
      source,
      temperature,
      ...(options.ledger ? { ledger: options.ledger } : {}),
    })
  const backrooms =
    options.backrooms ??
    buildBackrooms(config, {
      memory: options.memory,
      x: options.x,
      ...(options.exa ? { exa: options.exa } : {}),
      botUsername: options.botUsername,
      ...(options.ledger ? { ledger: options.ledger } : {}),
      ...(options.logger ? { logger: options.logger } : {}),
    })
  const loreText = readFileSync(config.NARRATIVE_LORE_PATH, 'utf8')
  const random = Math.random
  if (!options.media?.images) options.logger?.info({}, 'article: image generation is off; articles go without pictures')
  return new ArticlePipeline({
    backrooms,
    seed: async (topic) => {
      const news = await seedFromNews(topic, { ...(options.exa ? { exa: options.exa } : {}), random })
      if (news) return news
      const forums = (backrooms as Partial<Backrooms>).forums
      return forums ? seedFromForum(forums, topic, random) : undefined
    },
    topics: config.ARTICLE_NEWS_TOPICS,
    loreText,
    lore: parseLore(loreText),
    state: new FileNarrativeState(config.NARRATIVE_STATE_PATH),
    writer: model('article:writer', config.ARTICLE_WRITER_TEMPERATURE),
    editor: model('article:editor', 0.2),
    fixer: model('article:proof-fix', 0.4),
    humanizer: loadHumanizer(config.ARTICLE_HUMANIZER_PATH),
    ...(options.media?.images ? { images: options.media.images } : {}),
    ...(options.media?.reader ? { reader: options.media.reader } : {}),
    ...(options.media?.reference ? { reference: options.media.reference } : {}),
    store: options.media?.store ?? new MediaStore({ outputDir: config.MEDIA_OUTPUT_DIR }),
    budget: options.media?.budget ?? new MediaBudget({ imagesPerDay: 0, videosPerDay: 0 }),
    x: options.x,
    articles: options.articles ?? new FileArticleStore(config.ARTICLES_DIR),
    memory: options.memory,
    ...(options.critic ? { critic: options.critic } : {}),
    ...(options.safety ? { safety: options.safety } : {}),
    rooms: options.rooms ?? config.ARTICLE_ROOMS,
    sectionsMax: config.ARTICLE_SECTIONS_MAX,
    words: config.ARTICLE_WORDS,
    publish: options.publish,
    ...(options.structureOnly ? { structureOnly: true } : {}),
    ...(options.noImages ? { noImages: true } : {}),
    ...(options.noProof ? { noProof: true } : {}),
    ...(options.noFlow ? { noFlow: true } : {}),
    ...(options.from ? { from: options.from } : {}),
    ...(options.topic ? { topic: options.topic } : {}),
    ...(options.engine ? { engine: options.engine } : {}),
    ...(options.seedText ? { seedText: options.seedText } : {}),
    ...(options.brief ? { brief: options.brief } : {}),
    ...(options.logger ? { logger: options.logger } : {}),
  })
}
