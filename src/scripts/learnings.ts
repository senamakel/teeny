import 'dotenv/config'
import { readFileSync } from 'node:fs'
import { MongoPostLedger } from '../cadence/ledger.js'
import { MongoPeople } from '../cadence/people.js'
import { loadConfig } from '../config.js'
import { MongoJournal } from '../journal/journal.js'
import { reflect } from '../learnings/reflect.js'
import { OpenAITextModel } from '../media/reference.js'
import { CortexMemory } from '../memory/cortex.js'

/**
 * Rewrites the learnings brief now, rather than at LEARNINGS_CRON.
 *
 *   npm run learnings            (in the container: node dist/scripts/learnings.js)
 *   npm run learnings -- --print   also prints the brief it wrote
 *
 * The running bot reads the file live, so the next turn after this finishes
 * already carries the new brief; nothing needs restarting.
 */

const config = loadConfig()
const mongo = { uri: config.MONGO_URI, database: config.MONGO_DATABASE }
const ledger = new MongoPostLedger(mongo)
const journal = new MongoJournal(mongo)
const people = new MongoPeople(mongo)
const memory = new CortexMemory(
  { apiUrl: config.CORTEX_API_URL, actor: config.CORTEX_ACTOR, bearer: config.CORTEX_TOKEN },
  config.CORTEX_SCOPE,
  { recallMs: config.CORTEX_RECALL_TIMEOUT_MS, captureMs: config.CORTEX_CAPTURE_TIMEOUT_MS, personPass: config.CORTEX_PERSON_RECALL },
)

async function main(): Promise<void> {
  const result = await reflect({
    path: config.LEARNINGS_PATH,
    text: new OpenAITextModel({
      baseUrl: config.LLM_BASE_URL,
      apiKey: config.LLM_API_KEY,
      model: config.LLM_MODEL,
      source: 'learnings',
      timeoutMs: config.ARTICLE_MODEL_TIMEOUT_SECONDS * 1_000,
    }),
    ledger,
    journal,
    people,
    memory,
    botUsername: config.X_BOT_USERNAME,
    days: config.LEARNINGS_DAYS,
    maxChars: config.LEARNINGS_MAX_CHARS,
    logger: { info: (object, message) => console.log(message, object), warn: (object, message) => console.warn(message, object) },
  })
  console.log(result)
  if (process.argv.includes('--print') && result.outcome === 'written') {
    console.log(readFileSync(config.LEARNINGS_PATH, 'utf8'))
  }
}

main()
  .catch((error: unknown) => {
    console.error(error)
    process.exitCode = 1
  })
  .finally(() => Promise.all([ledger.close(), journal.close(), people.close()]))
