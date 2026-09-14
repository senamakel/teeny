import 'dotenv/config'
import { loadPersona } from '../agent/soul.js'
import { XApiClient } from '../clients/x.js'
import { loadConfig } from '../config.js'
import { buildMedia } from '../media/wiring.js'
import { CortexMemory } from '../memory/cortex.js'
import { buildMorningPipeline } from '../morning/wiring.js'
import { renderStories } from '../morning/news.js'
import { lexicalGate } from '../safety/gate.js'
import { AlgoCritic } from '../critic/algo.js'

/**
 * Rehearses the good-morning post without publishing.
 *
 *   npm run morning-dry-run              # the whole pipeline, dry: reads, draws, films, captions
 *   npm run morning-dry-run -- --publish # actually posts, if DRY_RUN=false
 *
 * It spends real generations: an image, and a video when VIDEO_GEN_ENABLED
 * is on, out of the same daily budget the service uses. Both are written to
 * MEDIA_OUTPUT_DIR so they can be looked at before anybody lets this thing
 * post on a timer.
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
const media = buildMedia(config, { log: (event, detail) => log(detail ?? {}, event) })
const pipeline = buildMorningPipeline(config, {
  memory,
  x,
  media,
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
  // The critic as the service has it, so a rehearsal shows the ranker's read
  // of the script and the caption and whether either was rewritten on it.
  ...(config.ALGO_CRITIC
    ? {
        critic: new AlgoCritic({
          baseUrl: config.LLM_BASE_URL,
          apiKey: config.LLM_API_KEY,
          model: config.ALGO_CRITIC_MODEL ?? config.LLM_MODEL,
        }),
      }
    : {}),
})

console.log(
  `\nRunning the morning pipeline as @${botUsername}, ${media?.videos ? 'with' : 'without'} video. ` +
    `${dryRun ? 'Nothing will be published.' : 'THIS WILL PUBLISH.'}\n`,
)

const result = await pipeline.run()

console.log(`\nNEWS (${result.stories.length} stories, ${result.droppedAsPolitical} dropped as political)`)
console.log(renderStories(result.stories))
console.log('\nPICK')
console.log(JSON.stringify(result.pick, null, 2))
if (result.story) console.log(`  from: ${result.story.url}`)
console.log('\nSEED')
console.log(result.seed ? `${result.seed.url}\n${result.seed.look}` : '(no reference sheet)')
console.log('\nCONCEPTS')
for (const concept of result.scene.concepts) console.log(`  - ${concept}`)
console.log('\nSCENE')
console.log(result.scene.scene)
console.log(`cue: ${result.scene.storyCue}`)
console.log(`alt: ${result.scene.alt}`)
console.log(`\nIMAGE: ${result.image ? `${result.image.id} at ${result.image.path ?? '(not saved)'}` : '(none)'}`)
console.log('\nSCRIPT')
console.log(result.script ? `${result.script.script}\n\nsummary: ${result.script.summary}` : '(none)')
console.log(`\nVIDEO: ${result.video ? `${result.video.id} at ${result.video.path ?? '(not saved)'}` : '(none)'}`)
console.log(`\nCAPTION (${result.caption.length} chars, ${result.attached} attached)\n`)
console.log(result.caption)
console.log(`\nOUTCOME: ${result.outcome}${result.postedId ? ` (post ${result.postedId})` : ''}`)
if (result.problems.length > 0) {
  console.log('\nPROBLEMS')
  for (const problem of result.problems) console.log(`  - ${problem}`)
}
console.log()
process.exit(0)
