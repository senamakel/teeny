import { renderPersona, type Persona } from '../agent/soul.js'
import { createExaClient } from '../agent/tools/exa.js'
import { buildBackrooms, seedFromForum, seedFromMemory, seedFromNews } from '../backrooms/wiring.js'
import type { Config } from '../config.js'
import type { InferenceLedger } from '../ledger/ledger.js'
import type { SafetyGate } from '../safety/gate.js'
import type { Memory, XTools } from '../types.js'
import { LorePipeline, type SeedSource } from './pipeline.js'

/**
 * Builds the lore pipeline from configuration. Shared by the service and
 * the rehearsal script, so a dry run rehearses the pipeline that will post.
 */
export function buildLorePipeline(
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
    ledger?: InferenceLedger
    random?: () => number
  },
): LorePipeline {
  const exa = config.EXA_API_KEY ? createExaClient(config.EXA_API_KEY) : undefined
  const random = options.random ?? Math.random
  const backrooms = buildBackrooms(config, {
    memory: options.memory,
    x: { searchPosts: (query, searchOptions) => options.x.searchPosts(query, searchOptions) },
    ...(exa ? { exa } : {}),
    botUsername: options.botUsername,
    persona: renderPersona(options.persona),
    maxEmoji: config.MAX_EMOJI_PER_POST,
    ...(options.ledger ? { ledger: options.ledger } : {}),
    ...(options.logger ? { logger: options.logger } : {}),
  })
  // Today's order is a rotation of the configured one, so the first source
  // is not always the one that answers. The forum is a random post either way.
  const configured = config.LORE_SEEDS as SeedSource[]
  const start = Math.floor(random() * configured.length)
  const sources = [...configured.slice(start), ...configured.slice(0, start)]
  return new LorePipeline({
    backrooms,
    seeds: {
      news: (topic) => seedFromNews(topic, { ...(exa ? { exa } : {}), random }),
      forum: (topic) => seedFromForum(backrooms.forums, topic, random),
      memory: (topic) => seedFromMemory(options.memory, topic),
    },
    sources,
    topics: config.LORE_TOPICS,
    x: options.x,
    memory: options.memory,
    turns: config.LORE_BACKROOMS_TURNS,
    dryRun: options.dryRun,
    ...(options.allowPosting === false ? { allowPosting: false } : {}),
    random,
    ...(options.logger ? { logger: options.logger } : {}),
    ...(options.safety ? { safety: options.safety } : {}),
  })
}
