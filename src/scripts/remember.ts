import 'dotenv/config'
import { loadConfig } from '../config.js'
import { CortexMemory } from '../memory/cortex.js'

/**
 * Writes one durable note to CortexDB, the same way the agent's own
 * `remember` tool does, for facts the operator wants in memory without
 * waiting for a turn to store them.
 *
 *   npm run remember -- "Teeny is a meme trading bot on pump.fun and fomo ..." [--label identity ...]
 *
 * Memory is recalled on every turn, so a note written here is read back in
 * the account's public voice: write it as a fact about the account, not as an
 * operational aside (see `forget` for what happens otherwise).
 */

const config = loadConfig()
const argv = process.argv.slice(2)

const labels: string[] = []
const words: string[] = []
for (let index = 0; index < argv.length; index += 1) {
  const arg = argv[index]
  if (arg === '--label') {
    const label = argv[index + 1]
    if (label) labels.push(label)
    index += 1
  } else if (arg) {
    words.push(arg)
  }
}
const note = words.join(' ').trim()

if (!note) {
  console.error('usage: npm run remember -- "<note>" [--label <label>]...')
  process.exit(2)
}

const memory = new CortexMemory(
  { apiUrl: config.CORTEX_API_URL, actor: config.CORTEX_ACTOR, bearer: config.CORTEX_TOKEN },
  config.CORTEX_SCOPE,
)

const id = `note:${Date.now()}:${Math.random().toString(36).slice(2, 10)}`
await memory.captureActivity('agent_note', id, { note, labels })
console.log(`Stored as ${id} in ${config.CORTEX_SCOPE} at ${config.CORTEX_API_URL}`)
