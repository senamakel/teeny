import 'dotenv/config'
import { readFileSync } from 'node:fs'
import { XApiClient } from '../clients/x.js'
import { loadConfig } from '../config.js'
import { AlgoCritic, renderCritique } from '../critic/algo.js'
import { createExaClient } from '../agent/tools/exa.js'
import { buildMedia } from '../media/wiring.js'
import { CortexMemory } from '../memory/cortex.js'
import { buildNarrativePipeline } from '../narrative/wiring.js'
import { episodeSchema, renderTranscript } from '../narrative/stages.js'
import { lexicalGate } from '../safety/gate.js'

/**
 * Rehearses one episode of the serial without publishing.
 *
 *   npm run narrative-dry-run                       # rooms, cuts, scores, a picture, a clip
 *   npm run narrative-dry-run -- --room-only        # rooms, cuts, scores; nothing generated
 *   npm run narrative-dry-run -- --branches 2 --turns 4
 *   npm run narrative-dry-run -- --publish          # posts, if DRY_RUN=false, and moves the ledger
 *   npm run narrative-dry-run -- --cut data/cut.json # films a saved cut as it is, no rooms
 *   npm run narrative-dry-run -- --visitor zuck     # the producer brings that visitor in
 *
 * --room-only is the loop for working on the prompts: it prints every
 * transcript, every cut and every score, and costs text calls only. A full
 * run spends an image and, with VIDEO_GEN_ENABLED, a video from the same
 * daily budget the service uses; both land in MEDIA_OUTPUT_DIR.
 */

const config = loadConfig()
const argv = process.argv.slice(2)
const flag = (name: string): string | undefined => {
  const index = argv.indexOf(`--${name}`)
  return index >= 0 ? argv[index + 1] : undefined
}
const publish = argv.includes('--publish')
const roomOnly = argv.includes('--room-only')
const dryRun = !publish || config.DRY_RUN
const branches = flag('branches') ? Number(flag('branches')) : undefined
const turns = flag('turns') ? Number(flag('turns')) : undefined
const visitor = flag('visitor')
const cut = flag('cut') ? episodeSchema.parse(JSON.parse(readFileSync(flag('cut')!, 'utf8'))) : undefined

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
const media = buildMedia(config, { log: (event, detail) => log(detail ?? {}, event) })
const pipeline = buildNarrativePipeline(config, {
  memory,
  x,
  media,
  ...(config.EXA_API_KEY ? { exa: createExaClient(config.EXA_API_KEY) } : {}),
  botUsername,
  dryRun,
  ...(dryRun ? { allowPosting: false } : {}),
  ...(roomOnly ? { roomOnly: true } : {}),
  ...(cut ? { cut } : {}),
  ...(visitor ? { visitor } : {}),
  ...(branches ? { branches } : {}),
  ...(turns ? { turns } : {}),
  logger: { info: log },
  safety: lexicalGate({ extraAllowedLinks: config.LINK_ALLOWLIST }),
  ...(config.ALGO_CRITIC
    ? {
        critic: new AlgoCritic({
          baseUrl: config.LLM_BASE_URL,
          apiKey: config.LLM_API_KEY,
          model: config.ALGO_CRITIC_MODEL,
        }),
      }
    : {}),
})

console.log(
  `\nRunning the serial as @${botUsername}${roomOnly ? ', rooms only' : media?.videos ? ', with video' : ', without video'}. ` +
    `${dryRun ? 'Nothing will be published.' : 'THIS WILL PUBLISH.'}\n`,
)

const result = await pipeline.run()

console.log(`\nEPISODE ${result.episodeNumber}`)
console.log('\nWORLD STATE')
console.log(result.briefing)
console.log(`\nLIVE LORE: ${result.liveLore.join(', ') || '(none)'}`)
for (const branch of result.branches) {
  console.log(`\n${'='.repeat(72)}\nBRANCH ${branch.index + 1}${branch.rejected ? ` -- ${branch.rejected}` : ''}`)
  console.log('\nTHE ROOM\n')
  console.log(renderTranscript(branch.transcript) || '(nothing written)')
  if (branch.episode) {
    console.log('\nTHE CUT')
    console.log(JSON.stringify(branch.episode, null, 2))
  }
  if (branch.score) {
    console.log(`\nSCORE: ${JSON.stringify(branch.score.scores)} composite ${branch.composite}`)
    console.log(`  ${branch.score.reasons}`)
    if (branch.score.hardFails.length > 0) console.log(`  hard fails: ${branch.score.hardFails.join('; ')}`)
  }
  if (branch.critique) console.log(`\nRANKER'S READ\n${renderCritique(branch.critique)}`)
}
console.log(`\n${'='.repeat(72)}`)
console.log(`\nCHOSEN: ${result.chosen ? `branch ${result.chosen.index + 1} "${result.chosen.episode?.title}"` : '(none)'}`)
console.log(`\nSEED: ${result.seed ? `${result.seed.url}\n${result.seed.look}` : '(no reference sheet)'}`)
if (result.likeness) console.log(`\nLIKENESS of ${result.likeness.name} (from ${result.likeness.photoUrl})\n${result.likeness.description}`)
console.log(`\nIMAGE: ${result.image ? `${result.image.id} at ${result.image.path ?? '(not saved)'}${result.visitorAs ? ` (visitor as ${result.visitorAs})` : ''}` : '(none)'}`)
console.log(`\nVIDEO: ${result.video ? `${result.video.id} at ${result.video.path ?? '(not saved)'}` : '(none)'}`)
console.log(`\nCAPTION (${result.caption.length} chars, ${result.attached} attached)\n`)
console.log(result.caption)
if (result.chosen?.episode) console.log(`\nSTATE AFTER\n${result.chosen.episode.stateAfter}\nopen threads: ${result.chosen.episode.openThreads.join('; ') || '(none)'}`)
console.log(`\nOUTCOME: ${result.outcome}${result.postedId ? ` (post ${result.postedId})` : ''}`)
if (result.problems.length > 0) {
  console.log('\nPROBLEMS')
  for (const problem of result.problems) console.log(`  - ${problem}`)
}
console.log()
process.exit(0)
