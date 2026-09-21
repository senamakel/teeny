import { renderPersona, type Persona } from '../agent/soul.js'
import type { Config } from '../config.js'
import type { InferenceLedger } from '../ledger/ledger.js'
import { OpenAITextModel } from '../media/reference.js'
import type { SafetyGate } from '../safety/gate.js'
import type { Memory, XTools } from '../types.js'
import { QuotePipeline } from './pipeline.js'
import { FileQuoteLog } from './store.js'

/**
 * Builds the quote pipeline from configuration. Shared by the service and
 * the rehearsal script, so a dry run rehearses the pipeline that will post.
 */
export function buildQuotePipeline(
  config: Config,
  options: {
    memory: Memory
    x: XTools
    persona: Persona
    botUsername: string
    dryRun: boolean
    allowPosting?: boolean
    logger?: { info: (object: unknown, message?: string) => void }
    safety?: Pick<SafetyGate, 'outbound' | 'screen'>
    ledger?: InferenceLedger
  },
): QuotePipeline {
  return new QuotePipeline({
    x: options.x,
    text: new OpenAITextModel({
      baseUrl: config.LLM_BASE_URL,
      apiKey: config.LLM_API_KEY,
      model: config.LLM_MODEL,
      source: 'quote',
      ...(options.ledger ? { ledger: options.ledger } : {}),
    }),
    memory: options.memory,
    log: new FileQuoteLog(config.QUOTE_LOG_PATH),
    persona: renderPersona(options.persona),
    botUsername: options.botUsername,
    trustedHandles: config.TRUSTED_HANDLES,
    ownTokenAddresses: [...new Set([...config.OWN_TOKEN_ADDRESSES, ...(config.CHART_TOKEN_ADDRESS ? [config.CHART_TOKEN_ADDRESS] : [])])],
    ownSymbol: config.CHART_TOKEN_SYMBOL,
    query: config.QUOTE_QUERY,
    maxAgeMs: config.QUOTE_HOURS * 3_600_000,
    authorCooldownMs: config.QUOTE_AUTHOR_COOLDOWN_DAYS * 24 * 3_600_000,
    maxResults: config.QUOTE_POSTS_PER_SEARCH,
    maxEmoji: config.MAX_EMOJI_PER_POST,
    dryRun: options.dryRun,
    ...(options.allowPosting === false ? { allowPosting: false } : {}),
    ...(options.logger ? { logger: options.logger } : {}),
    ...(options.safety ? { safety: options.safety } : {}),
  })
}
