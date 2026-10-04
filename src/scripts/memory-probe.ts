import 'dotenv/config'
import { MongoPostLedger } from '../cadence/ledger.js'
import { MongoPeople } from '../cadence/people.js'
import { loadConfig } from '../config.js'
import { TurnContext } from '../memory/context.js'
import { CortexMemory } from '../memory/cortex.js'
import { checkCortex } from '../memory/health.js'
import { MongoMentionQueue, type Exchange } from '../queue/mentions.js'

/**
 * Measures whether recall works, against the live store, for real people.
 *
 *   npm run memory:probe -- [--sample 10] [--person <x-user-id>] [--json]
 *   (in the container: node dist/scripts/memory-probe.js --sample 10)
 *
 * For each of the account's most frequent correspondents who has been
 * answered before, it builds the turn context a reply to them would get and
 * checks three things:
 *
 * - exact: the turn shows the last exchanges with them, from the queue.
 * - recall: memory, asked about the person, brings back at least one of the
 *   last three things said in those exchanges. This is the test of memory
 *   itself: the exchanges are known to be stored, so a miss is retrieval.
 * - own posts: the turn shows the account's newest post from the ledger.
 *
 * Read-only: it recalls and reads, it never captures.
 */

const config = loadConfig()
const argv = process.argv.slice(2)
const flag = (name: string) => {
  const index = argv.indexOf(`--${name}`)
  return index >= 0 ? argv[index + 1] : undefined
}
const sample = Number(flag('sample') ?? 10)
const only = flag('person')
const asJson = argv.includes('--json')

const memory = new CortexMemory(
  { apiUrl: config.CORTEX_API_URL, actor: config.CORTEX_ACTOR, bearer: config.CORTEX_TOKEN },
  config.CORTEX_SCOPE,
  { recallMs: config.CORTEX_RECALL_TIMEOUT_MS, captureMs: config.CORTEX_CAPTURE_TIMEOUT_MS, personPass: config.CORTEX_PERSON_RECALL },
)
const mongo = { uri: config.MONGO_URI, database: config.MONGO_DATABASE }
const queue = new MongoMentionQueue(mongo)
const ledger = new MongoPostLedger(mongo)
const people = new MongoPeople(mongo)

/** Loose enough to survive whitespace and emoji, strict enough to mean it. */
function fingerprint(text: string): string {
  return text
    .toLowerCase()
    .replace(/https?:\/\/\S+/g, '')
    .replace(/@\w+/g, '')
    .replace(/[^a-z0-9 ]+/g, ' ')
    .replace(/\s+/g, ' ')
    .trim()
    .slice(0, 48)
}

function recalls(block: string, exchanges: Exchange[]): boolean {
  const haystack = block.toLowerCase().replace(/@\w+/g, '').replace(/[^a-z0-9 ]+/g, ' ').replace(/\s+/g, ' ')
  return exchanges
    .slice(0, 3)
    .flatMap((exchange) => [exchange.incoming, exchange.reply])
    .map(fingerprint)
    .filter((needle) => needle.length >= 16)
    .some((needle) => haystack.includes(needle.slice(0, 32)))
}

async function timed<T>(work: () => Promise<T>): Promise<{ value?: T; ms: number; error?: string }> {
  const started = Date.now()
  try {
    return { value: await work(), ms: Date.now() - started }
  } catch (error) {
    return { ms: Date.now() - started, error: error instanceof Error ? error.message : String(error) }
  }
}

async function main(): Promise<void> {
  const health = await checkCortex(config.CORTEX_API_URL)
  const newest = (await ledger.since(new Date(Date.now() - config.MEMORY_OWN_POSTS_HOURS * 3_600_000).toISOString(), 1))[0]

  const candidates = only
    ? [{ userId: only, username: undefined as string | undefined }]
    : (await people.top(sample * 4)).map((person) => ({ userId: person.userId, username: person.username }))

  const rows: Array<Record<string, unknown>> = []
  for (const candidate of candidates) {
    if (rows.length >= sample) break
    const exchanges = await queue.exchangesWith(candidate.userId, config.MEMORY_EXCHANGES)
    if (exchanges.length === 0 && !only) continue
    const person = { id: `user:x-${candidate.userId}`, ...(candidate.username ? { username: candidate.username } : {}) }

    const context = new TurnContext({
      memory,
      ownPosts: ledger,
      exchanges: (authorId, limit, options) => queue.exchangesWith(authorId, limit, options),
      links: config.PEOPLE_LINKS,
      limits: {
        exchanges: config.MEMORY_EXCHANGES,
        ownPostsHours: config.MEMORY_OWN_POSTS_HOURS,
        ownPostsLimit: config.MEMORY_OWN_POSTS_LIMIT,
      },
    })
    const turn = await timed(() =>
      context.gather({ query: `A reply to @${candidate.username ?? candidate.userId}: ${exchanges[0]?.incoming ?? 'gm'}`, person, authorId: candidate.userId }),
    )
    // Memory alone, asked about the person: the part that can actually miss.
    const about = await timed(() => memory.recall('What has this person said, and what did you say back?', { person }))

    rows.push({
      person: `@${candidate.username ?? candidate.userId}`,
      exchanges: exchanges.length,
      exact: Boolean(turn.value?.text.includes('EXCHANGES WITH')),
      recall: about.value === undefined ? 'error' : recalls(about.value, exchanges) ? 'hit' : 'miss',
      recallMs: about.ms,
      // Over the cap means the store ignored its budget and the client cut it.
      packChars: about.value?.length ?? 0,
      turnMs: turn.ms,
      ownPosts: newest?.text ? Boolean(turn.value?.text.includes((newest.text ?? '').slice(0, 40).replace(/\s+/g, ' '))) : 'none in window',
      degraded: turn.value?.degraded.join(',') || (turn.error ? 'all' : ''),
      ...(about.error ? { error: about.error.slice(0, 120) } : {}),
    })
  }

  const scored = rows.filter((row) => row.recall !== 'error' && Number(row.exchanges) > 0)
  const hits = scored.filter((row) => row.recall === 'hit').length
  const summary = {
    store: { reachable: health.reachable, degraded: health.degraded, embeddings: health.embeddings, provider: health.provider },
    people: rows.length,
    recallHitRate: scored.length ? `${hits}/${scored.length}` : 'n/a',
    exactHistory: `${rows.filter((row) => row.exact).length}/${rows.length}`,
    medianRecallMs: median(rows.map((row) => Number(row.recallMs))),
    medianTurnMs: median(rows.map((row) => Number(row.turnMs))),
  }

  if (asJson) {
    console.log(JSON.stringify({ summary, rows }, null, 2))
  } else {
    console.table(rows)
    console.log(summary)
  }
}

function median(values: number[]): number | undefined {
  const sorted = values.filter(Number.isFinite).sort((a, b) => a - b)
  return sorted.length ? sorted[Math.floor(sorted.length / 2)] : undefined
}

main()
  .catch((error: unknown) => {
    console.error(error)
    process.exitCode = 1
  })
  .finally(() => Promise.all([queue.close(), ledger.close(), people.close()]))
