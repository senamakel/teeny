import 'dotenv/config'
import { loadPersona } from '../agent/soul.js'
import { XApiClient } from '../clients/x.js'
import { loadConfig } from '../config.js'
import { buildLorePipeline } from '../lore/wiring.js'
import { CortexMemory } from '../memory/cortex.js'
import { lexicalGate } from '../safety/gate.js'

/**
 * Rehearses the lore post without publishing.
 *
 *   npm run lore-dry-run              # seed, room, thought, post; post nothing
 *   npm run lore-dry-run -- --publish # actually posts, if DRY_RUN=false
 *
 * The room is real and is written to BACKROOMS_DIR either way, so the
 * transcript can be read before anybody lets this thing post on a timer.
 */

const config = loadConfig()
const argv = process.argv.slice(2)
const publish = argv.includes('--publish')
const dryRun = !publish || config.DRY_RUN

const memory = new CortexMemory(
  { apiUrl: config.CORTEX_API_URL, actor: config.CORTEX_ACTOR, bearer: config.CORTEX_TOKEN },
  config.CORTEX_SCOPE,
)
const x = new XApiClient({
  username: config.X_BOT_USERNAME,
  apiKey: config.X_API_KEY,
  apiSecret: config.X_API_SECRET,
  accessToken: config.X_ACCESS_TOKEN,
  accessTokenSecret: config.X_ACCESS_TOKEN_SECRET,
  ...(config.X_BEARER_TOKEN ? { bearerToken: config.X_BEARER_TOKEN } : {}),
})
const botUsername = await x
  .identity()
  .then((identity) => identity.username)
  .catch(() => config.X_BOT_USERNAME)

const log = (object: unknown, message?: string): void => {
  console.log(`  ${message ?? ''} ${JSON.stringify(object)}`)
}
const pipeline = buildLorePipeline(config, {
  memory,
  x,
  persona: loadPersona({
    soulPath: config.SOUL_PATH,
    contextPath: config.CONTEXT_PATH,
    skillsDir: config.SKILLS_DIR,
  }),
  botUsername,
  dryRun,
  ...(dryRun ? { allowPosting: false } : {}),
  logger: { info: log },
  // The lexicon and the allowlist, as the service has them; no judge and no
  // hold log, since this is a script and --publish can post for real.
  safety: lexicalGate({ extraAllowedLinks: config.LINK_ALLOWLIST }),
})

console.log(
  `\nRunning the lore pipeline as @${botUsername}, seeds from ${config.LORE_SEEDS.join('/')}, ${config.LORE_BACKROOMS_TURNS} turns. ` +
    `${dryRun ? 'Nothing will be published.' : 'THIS WILL PUBLISH.'}\n`,
)

const result = await pipeline.run()

if (result.seed) {
  console.log(`\nSEED (${result.from?.source} on "${result.from?.topic}"; ${result.seed.kind}${result.seed.source ? `, ${result.seed.source}` : ''})`)
  console.log(`  ${result.seed.title ? `${result.seed.title} — ` : ''}${result.seed.text.slice(0, 300).replace(/\s+/g, ' ')}`)
}
if (result.room) {
  console.log(`\nTHE ROOM: ${result.room.saved}`)
  console.log(`\nTHE THOUGHT\n  ${result.room.thought.premise}`)
  if (result.room.thought.wire) console.log(`  the wire: ${result.room.thought.wire}`)
  console.log(`  ${result.room.thought.thought}`)
  if (result.room.thought.objects.length > 0) console.log(`  objects: ${result.room.thought.objects.join(', ')}`)
}
if (result.post) {
  console.log(`\nREASONS: ${result.post.reasons}`)
  if (result.post.continues) console.log(`CONTINUES: ${result.post.continues}`)
}
console.log(`\nPOST (${result.text.length} chars)\n`)
console.log(result.text)
console.log(`\nOUTCOME: ${result.outcome}${result.postedId ? ` (post ${result.postedId})` : ''}`)
if (result.problems.length > 0) {
  console.log('\nPROBLEMS')
  for (const problem of result.problems) console.log(`  - ${problem}`)
}
console.log()
process.exit(0)
