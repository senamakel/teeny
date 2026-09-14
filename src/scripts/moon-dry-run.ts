import 'dotenv/config'
import { loadPersona } from '../agent/soul.js'
import { XApiClient } from '../clients/x.js'
import { loadConfig } from '../config.js'
import { CortexMemory } from '../memory/cortex.js'
import { buildMoonPipeline } from '../moon/wiring.js'
import { multiple } from '../moon/svg.js'
import { lexicalGate } from '../safety/gate.js'

/**
 * Rehearses the moon chart without publishing.
 *
 *   npm run moon-dry-run              # read the board, draw, caption; post nothing
 *   npm run moon-dry-run -- --publish # actually posts, if DRY_RUN=false
 *
 * The picture is written to MOON_OUTPUT_DIR either way, so it can be looked
 * at before anybody lets this thing post on a timer.
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
const pipeline = buildMoonPipeline(config, {
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
  `\nRunning the moon pipeline as @${botUsername} on CoinGecko's "${config.MOON_CATEGORY}". ` +
    `${dryRun ? 'Nothing will be published.' : 'THIS WILL PUBLISH.'}\n`,
)

const result = await pipeline.run()

console.log(`\nBOARD (${result.board.length} coins)`)
for (const coin of result.board) console.log(`  ${coin.symbol.padEnd(10)} $${Math.round(coin.marketCapUsd).toLocaleString('en-US')}`)
console.log(`\nTOKEN: $${result.token.symbol} at $${result.token.priceUsd}, cap $${Math.round(result.token.marketCapUsd).toLocaleString('en-US')}`)
console.log(`\nYARDSTICK: $${result.target.coin.symbol} (${result.target.coin.name}) -> $${result.target.impliedPriceUsd} (${multiple(result.target.multiple)})`)
if (result.excluded.length > 0) console.log(`  skipped as recently used: ${result.excluded.join(', ')}`)
console.log('\nLADDER')
for (const entry of result.ladder) console.log(`  ${entry.coin.symbol.padEnd(10)} ${multiple(entry.multiple).padStart(8)}  $${entry.impliedPriceUsd}`)
console.log(`\nPICTURE: ${result.path ?? `${result.png.byteLength} bytes (not saved)`}`)
console.log(`\nCAPTION (${result.caption.length} chars)\n`)
console.log(result.caption)
console.log(`\nOUTCOME: ${result.outcome}${result.postedId ? ` (post ${result.postedId})` : ''}`)
if (result.problems.length > 0) {
  console.log('\nPROBLEMS')
  for (const problem of result.problems) console.log(`  - ${problem}`)
}
console.log()
process.exit(0)
