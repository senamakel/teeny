import { z } from 'zod'

const booleanFromString = z
  .enum(['true', 'false'])
  .default('true')
  .transform((value) => value === 'true')

/**
 * A value that may be left blank.
 *
 * `.optional()` alone is not enough: it permits the key to be absent, but a
 * filled-in .env spells "absent" as `KEY=`, and an empty string is a string,
 * which then fails the format check and takes the process down at boot. Every
 * optional value with a format has to accept the empty spelling too.
 */
function blankable<T extends z.ZodType>(schema: T) {
  return z.preprocess((value) => (value === '' ? undefined : value), schema.optional())
}

/** For switches whose safe position is off rather than on. */
const offByDefault = z
  .enum(['true', 'false'])
  .default('false')
  .transform((value) => value === 'true')

const schema = z.object({
  PORT: z.coerce.number().int().min(1).max(65535).default(3001),
  LOG_LEVEL: z.string().default('info'),
  DRY_RUN: booleanFromString,
  REPLY_CRON: z.string().default('*/5 * * * *'),
  MAX_REPLIES_PER_RUN: z.coerce.number().int().min(1).max(100).default(5),
  X_BOT_USERNAME: z.string().min(1),
  X_API_KEY: z.string().min(1),
  X_API_SECRET: z.string().min(1),
  X_ACCESS_TOKEN: z.string().min(1),
  X_ACCESS_TOKEN_SECRET: z.string().min(1),
  X_BEARER_TOKEN: z.string().optional(),
  CORTEX_API_URL: z.url().default('https://api-v1.cortexdb.ai'),
  CORTEX_ACTOR: z.string().min(1).default('agent:teeny'),
  CORTEX_SCOPE: z.string().min(1).default('agent:teeny'),
  CORTEX_TOKEN: z.string().min(1),
  LLM_BASE_URL: z.url().default('https://api.openai.com/v1'),
  LLM_API_KEY: z.string().min(1),
  LLM_MODEL: z.string().min(1),
  EXA_API_KEY: z.string().min(1).optional(),
  AGENT_MODE: z.enum(['langgraph', 'simple']).default('langgraph'),
  AGENT_MAX_STEPS: z.coerce.number().int().min(1).max(100).default(24),
  AGENT_API_TOKEN: z.string().min(16).optional(),
  SOUL_PATH: z.string().default('SOUL.md'),
  CONTEXT_PATH: z.string().default('CONTEXT.md'),
  SKILLS_DIR: z.string().default('skills'),
  MAX_EMOJI_PER_POST: z.coerce.number().int().min(0).max(10).default(1),
  GIPHY_API_KEY: z.string().min(1).optional(),
  GIPHY_CHANNEL: z.string().min(1).default('TinyHumansAI'),
  // X bills per post retrieved, so both of these are cost dials.
  MENTION_LOOKBACK_MINUTES: z.coerce.number().int().min(1).max(10_080).default(15),
  X_SEARCH_MAX_RESULTS: z.coerce.number().int().min(10).max(100).default(10),
  // `all` runs every role in one process. Split them to scale or restart the
  // reply side without pausing ingestion, which is the half with a deadline.
  SERVICE_ROLE: z.enum(['all', 'ingest', 'reply', 'api']).default('all'),
  INGEST_CRON: z.string().default('*/5 * * * *'),
  // `worker` drains the queue continuously, which is what makes a webhook
  // delivery turn into a reply in seconds rather than by the next tick.
  // `cron` keeps the old timer for a quiet account that would rather poll.
  REPLY_MODE: z.enum(['worker', 'cron']).default('worker'),
  // How long the worker waits after finding an empty queue. One indexed query
  // per tick, so this is cheap; it is latency, not load.
  REPLY_IDLE_MS: z.coerce.number().int().min(250).max(60_000).default(2_000),
  // The least time between two published replies. A giveaway had the account
  // answering a hundred people in an hour and X throttled its reach for it;
  // a minute apart, the same replies read as a conversation. Zero disables it.
  REPLY_MIN_INTERVAL_MS: z.coerce.number().int().min(0).max(3_600_000).default(60_000),
  MONGO_URI: z.string().min(1).default('mongodb://mongo:27017'),
  MONGO_DATABASE: z.string().min(1).default('teeny'),
  MENTIONS_COLLECTION: z.string().min(1).default('mentions'),
  // How much of the thread is fetched before drafting. Zero disables it, and
  // the reply then sees only the post that mentioned the account. Every post
  // retrieved is billed and X's floor on a search is ten, so this is the most
  // expensive part of a reply: at five replies a run it is the difference
  // between fifty posts and none.
  // The account's own token contracts, comma separated. Matched against every
  // post before drafting, because a model will not reliably match a contract
  // address against a long persona document -- and getting this wrong means
  // calling your own token somebody else's shill, in public.
  OWN_TOKEN_ADDRESSES: z
    .string()
    .default('')
    .transform((value) => value.split(',').map((entry) => entry.trim()).filter(Boolean)),
  THREAD_CONTEXT_MAX_POSTS: z.coerce.number().int().min(0).max(100).default(10),

  // --- the wallet ----------------------------------------------------------
  // Nothing reads these today. The agent has no airdrop tool: it was added and
  // removed on 2026-09-08, and `test/capabilities.test.ts` explains why. The
  // settings stay because `src/airdrop/` and `src/clients/wallet.ts` stay, and
  // re-wiring a tool would need them back exactly as they were.
  AIRDROP_ENABLED: offByDefault,
  AIRDROP_DRY_RUN: booleanFromString,
  AIRDROP_PRIVATE_KEY: blankable(
    z.string().regex(/^(0x)?[0-9a-fA-F]{64}$/, 'must be a 32-byte hex private key'),
  ),
  AIRDROP_TOKEN_ADDRESS: blankable(
    z.string().regex(/^0x[0-9a-fA-F]{40}$/, 'must be a contract address'),
  ),
  AIRDROP_TOKEN_SYMBOL: z.string().min(1).default('TINY'),
  AIRDROP_RPC_URL: blankable(z.string()),
  AIRDROPS_COLLECTION: z.string().min(1).default('airdrops'),
  // The ceilings. A turn cannot raise any of them by asking.
  AIRDROP_MAX_PER_AIRDROP: z.coerce.number().int().min(1).default(100),
  // What the account has publicly promised. The model still chooses, but it is
  // told the number it has committed to in front of everybody: the campaign
  // post says 100 and it was sending 10, which is worse than sending nothing.
  AIRDROP_STANDARD_AMOUNT: z.coerce.number().int().min(1).default(100),
  AIRDROP_MAX_PER_DAY: z.coerce.number().int().min(1).default(1000),
  AIRDROP_MAX_PER_RECIPIENT: z.coerce.number().int().min(1).default(1),
  // Off, this pays any address anywhere in the turn -- including one that came
  // from a search result or a recalled memory rather than from the person
  // being answered. Leave it on unless you know why you are turning it off.
  AIRDROP_REQUIRE_ADDRESS_IN_POST: booleanFromString,
  // A weak signal deliberately: a follow costs nothing and a farm can buy a
  // thousand. It filters drive-by begging; the caps are what make a bad day
  // survivable.
  AIRDROP_REQUIRE_FOLLOW: booleanFromString,
  RECIPIENTS_COLLECTION: z.string().min(1).default('recipients'),
})

export type Config = z.infer<typeof schema>

export function loadConfig(environment: NodeJS.ProcessEnv = process.env): Config {
  return schema.parse(environment)
}
