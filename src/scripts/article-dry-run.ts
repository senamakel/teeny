import 'dotenv/config'
import { readFileSync } from 'node:fs'
import { createExaClient } from '../agent/tools/exa.js'
import { renderStructure } from '../article/stages.js'
import { buildArticlePipeline } from '../article/wiring.js'
import type { SavedExtras } from '../backrooms/store.js'
import type { BackroomsRun } from '../backrooms/types.js'
import { buildBackrooms } from '../backrooms/wiring.js'
import { XApiClient } from '../clients/x.js'
import { loadConfig } from '../config.js'
import { AlgoCritic, renderCritique } from '../critic/algo.js'
import { buildMedia } from '../media/wiring.js'
import { CortexMemory } from '../memory/cortex.js'
import { lexicalGate } from '../safety/gate.js'

/**
 * Rehearses one article without publishing, stopping where it is told.
 *
 *   npm run article-dry-run -- --structure-only              # rooms, the plan, its check; nothing written
 *   npm run article-dry-run -- --no-images                   # the whole text, proof-read; nothing drawn
 *   npm run article-dry-run                                  # the text and the pictures
 *   npm run article-dry-run -- --from data/backrooms/<run>.json   # a saved room; no seed, no rooms
 *   npm run article-dry-run -- --engine "the fridge rate"    # what tonight is about
 *   npm run article-dry-run -- --topic "ai agents"           # what the wire is asked for
 *   npm run article-dry-run -- --rooms 1 --no-proof
 *   npm run article-dry-run -- --publish                     # publishes, if DRY_RUN=false
 *
 * Every run is saved under ARTICLES_DIR as a record with each stage's
 * output, so a rehearsal can be read file by file. Pictures spend the same
 * daily image budget the service uses.
 */

const config = loadConfig()
const argv = process.argv.slice(2)
const flag = (name: string): string | undefined => {
  const index = argv.indexOf(`--${name}`)
  return index >= 0 && !argv[index + 1]?.startsWith('--') ? argv[index + 1] : undefined
}
const has = (name: string): boolean => argv.includes(`--${name}`)
const publish = has('publish') && !config.DRY_RUN

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
const exa = config.EXA_API_KEY ? createExaClient(config.EXA_API_KEY) : undefined
const media = has('no-images') ? undefined : buildMedia(config, { log: (event, detail) => log(detail ?? {}, event) })

let from: { run: BackroomsRun; thought: NonNullable<SavedExtras['thought']>; judge?: NonNullable<SavedExtras['judge']>; path: string } | undefined
if (flag('from')) {
  const path = flag('from')!
  const { stats: _stats, judge, thought, variants: _variants, ...run } = JSON.parse(readFileSync(path, 'utf8')) as BackroomsRun & SavedExtras
  const backrooms = buildBackrooms(config, { memory, x, ...(exa ? { exa } : {}), botUsername, logger: { info: log } })
  from = { run, thought: thought ?? (await backrooms.distill(run)), ...(judge ? { judge } : {}), path }
}

const pipeline = buildArticlePipeline(config, {
  memory,
  x,
  media,
  ...(exa ? { exa } : {}),
  botUsername,
  publish,
  ...(from ? { from } : {}),
  ...(flag('rooms') ? { rooms: Number(flag('rooms')) } : {}),
  ...(has('structure-only') ? { structureOnly: true } : {}),
  ...(has('no-images') ? { noImages: true } : {}),
  ...(has('no-proof') ? { noProof: true } : {}),
  ...(flag('topic') ? { topic: flag('topic')! } : {}),
  ...(flag('engine') ? { engine: flag('engine')! } : {}),
  logger: { info: log },
  safety: lexicalGate({ extraAllowedLinks: config.LINK_ALLOWLIST }),
  ...(config.ALGO_CRITIC
    ? { critic: new AlgoCritic({ baseUrl: config.LLM_BASE_URL, apiKey: config.LLM_API_KEY, model: config.ALGO_CRITIC_MODEL }) }
    : {}),
})

console.log(
  `\nWriting an article as @${botUsername}${has('structure-only') ? ', the plan only' : has('no-images') ? ', without pictures' : ''}. ` +
    `${publish ? 'THIS WILL PUBLISH.' : 'Nothing will be published.'}\n`,
)

const result = await pipeline.run()

console.log(`\nENGINE: ${result.engine ?? '(the room drew its own)'}`)
console.log(`\nSEED: ${result.seed ? `(${result.seed.kind}${result.seed.source ? `, ${result.seed.source}` : ''}) ${result.seed.title ? `${result.seed.title} — ` : ''}${result.seed.text.slice(0, 300)}` : '(none)'}`)
console.log('\nROOMS')
for (const room of result.rooms) {
  console.log(`  ${room.index + 1}. ${room.runId ?? '(no run)'}${room.total !== undefined ? ` scored ${room.total}/21` : ''}${room.rejected ? ` -- ${room.rejected}` : ''}${room.saved ? `\n     ${room.saved}` : ''}`)
}
if (result.thought) {
  console.log('\nTHE THOUGHT')
  console.log(`  ${result.thought.premise}`)
  for (const line of result.thought.thought.split('\n')) console.log(`  ${line}`)
}
if (result.structure) {
  console.log(`\n${'='.repeat(72)}\nTHE PLAN\n`)
  console.log(renderStructure(result.structure))
  if (result.structureCheck) {
    console.log(`\nTHE EDITOR: ${result.structureCheck.ok ? 'ok' : 'not ok'}`)
    for (const line of result.structureCheck.problems) console.log(`  - ${line}`)
    for (const line of result.structureCheck.fixes) console.log(`  fix: ${line}`)
  }
}
if (result.proof) {
  console.log(`\n${'='.repeat(72)}\nTHE PROOF-READER: ${result.proof.review.verdict}, ${result.proof.review.findings.length} finding(s), fix ${result.proof.applied ? 'applied' : result.proof.refused ? `refused (${result.proof.refused})` : 'not needed'}`)
  console.log(`  voice: ${result.proof.review.voice}`)
  for (const finding of result.proof.review.findings) console.log(`  - [${finding.section || '?'}] ${finding.pattern}: "${finding.quote}" → ${finding.fix}`)
  if (result.proof.after) console.log(`  after the fix: ${result.proof.after.verdict}, ${result.proof.after.findings.length} finding(s)`)
}
if (result.markdown) {
  console.log(`\n${'='.repeat(72)}\n# ${result.structure?.title ?? ''}\n`)
  console.log(result.markdown)
  console.log(`\n(${result.wordCount} words)`)
}
if (result.critique) console.log(`\nRANKER'S READ\n${renderCritique(result.critique)}`)
console.log(`\nCOVER: ${result.cover ? `${result.cover.id} at ${result.cover.path ?? '(not saved)'}` : '(none)'}`)
for (const image of result.images) console.log(`IMAGE ${image.key}: ${image.media.id} at ${image.media.path ?? '(not saved)'}`)
if (result.contentState) console.log(`\nCONTENT STATE: ${result.contentState.blocks.length} blocks, ${result.contentState.entities.length} entities`)
console.log(`\nOUTCOME: ${result.outcome}${result.postId ? ` (post ${result.postId}, article ${result.xArticleId})` : ''}`)
if (result.slug) console.log(`SAVED: ${config.ARTICLES_DIR}/${result.slug}/article.json`)
if (result.problems.length > 0) {
  console.log('\nPROBLEMS')
  for (const problem of result.problems) console.log(`  - ${problem}`)
}
console.log()
process.exit(0)
