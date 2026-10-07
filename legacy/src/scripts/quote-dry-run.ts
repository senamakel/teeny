import 'dotenv/config'
import { loadPersona } from '../agent/soul.js'
import { XApiClient } from '../clients/x.js'
import { loadConfig } from '../config.js'
import { CortexMemory } from '../memory/cortex.js'
import { buildQuotePipeline } from '../quote/wiring.js'
import { lexicalGate } from '../safety/gate.js'

/**
 * Rehearses the quote post without publishing.
 *
 *   npm run quote-dry-run              # search, filter, pick, write; post nothing
 *   npm run quote-dry-run -- --publish # actually posts, if DRY_RUN=false
 *
 * The two searches are real and X bills them. The pick is logged to
 * QUOTE_LOG_PATH either way, so a rehearsal followed by a real run quotes
 * two different posts; delete the entry from the file to let the real run
 * have the rehearsed one.
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
const pipeline = buildQuotePipeline(config, {
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
  `\nRunning the quote pipeline as @${botUsername} over "${config.QUOTE_QUERY}" and the mentions, last ${config.QUOTE_HOURS}h. ` +
    `${dryRun ? 'Nothing will be published.' : 'THIS WILL PUBLISH.'}\n`,
)

const result = await pipeline.run()

console.log(`\nSEARCHED ${result.searched} posts; ${result.candidates.length} candidates, ${result.skipped.length} skipped`)
for (const entry of result.skipped) console.log(`  - ${entry.id} ${entry.author ? `@${entry.author}` : ''}: ${entry.reason}`)
console.log('\nCANDIDATES')
for (const candidate of result.candidates) {
  const { post } = candidate
  console.log(`  ${post.id} @${post.authorUsername ?? '?'} [${candidate.source}${candidate.trusted ? ', team' : ''}, score ${candidate.score}]`)
  console.log(`    ${post.text.replace(/\s+/g, ' ').trim()}`)
}
if (result.chosen) {
  console.log(`\nCHOSEN: ${result.chosen.id} by @${result.chosen.authorUsername ?? '?'}`)
  console.log(`  ${result.chosen.text.replace(/\s+/g, ' ').trim()}`)
}
console.log(`\nWHY: ${result.why}`)
console.log(`\nTEXT (${result.text.length} chars)\n`)
console.log(result.text)
console.log(`\nOUTCOME: ${result.outcome}${result.postedId ? ` (post ${result.postedId})` : ''}`)
if (result.problems.length > 0) {
  console.log('\nPROBLEMS')
  for (const problem of result.problems) console.log(`  - ${problem}`)
}
console.log()
process.exit(0)
