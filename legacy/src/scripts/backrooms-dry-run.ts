import 'dotenv/config'
import { createExaClient } from '../agent/tools/exa.js'
import { buildBackrooms, seedFromForum, seedFromMemory, seedFromMention, seedFromNews } from '../backrooms/wiring.js'
import { renderTurn } from '../backrooms/evaluate.js'
import { rng } from '../backrooms/tokens.js'
import type { Seed } from '../backrooms/types.js'
import { readFileSync } from 'node:fs'
import { FileBackroomsStore, type SavedExtras } from '../backrooms/store.js'
import { renderVariants } from '../backrooms/variants.js'
import type { BackroomsRun } from '../backrooms/types.js'
import { XApiClient } from '../clients/x.js'
import { loadConfig } from '../config.js'
import { CortexMemory } from '../memory/cortex.js'
import { loadPersona, renderPersona } from '../agent/soul.js'

/**
 * Opens the backrooms on something and prints what came out.
 *
 *   npm run backrooms -- --seed "a car pulled up outside the lab at 3am"
 *   npm run backrooms -- --news "ai agents"          # a story from the wire as the seed
 *   npm run backrooms -- --memory "the door"          # a recall as the seed
 *   npm run backrooms -- --mention                    # the latest mention as the seed
 *   npm run backrooms -- --forum "ai escaped"         # a random reddit/4chan post as the seed
 *
 *   --turns 12 --runs 1 --agents teeny,swarm --temps 1.0,1.2 --rng 7 --budget 9000
 *   --tools 3 --drift 4 --score --no-distill --variants
 *   npm run backrooms -- --distill data/backrooms/<run>.json    # the thought again, on a saved run
 *   npm run backrooms -- --variants data/backrooms/<run>.json   # article, tweets, video, code from a saved run
 *   npm run backrooms -- --news --gm                             # the good-morning post from the thought
 *   npm run backrooms -- --gm data/backrooms/<run>.json          # the good morning again, on a saved run
 *
 * Nothing posts. Every run is written to BACKROOMS_DIR as .json and .md.
 * --score asks a cold model to grade the transcript for craziness; the same
 * --rng with different --temps is how settings are compared.
 */

const config = loadConfig()
const argv = process.argv.slice(2)
const flag = (name: string): string | undefined => {
  const index = argv.indexOf(`--${name}`)
  return index >= 0 && !argv[index + 1]?.startsWith('--') ? argv[index + 1] : undefined
}
const has = (name: string): boolean => argv.includes(`--${name}`)
const number = (name: string): number | undefined => (flag(name) ? Number(flag(name)) : undefined)

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
const exa = config.EXA_API_KEY ? createExaClient(config.EXA_API_KEY) : undefined

const log = (object: unknown, message?: string): void => {
  console.log(`  ${message ?? ''} ${JSON.stringify(object)}`)
}
const persona = renderPersona(loadPersona({ soulPath: config.SOUL_PATH, contextPath: config.CONTEXT_PATH, skillsDir: config.SKILLS_DIR }))
const backrooms = buildBackrooms(config, {
  memory,
  x,
  ...(exa ? { exa } : {}),
  botUsername,
  persona,
  logger: { info: log },
})

const printThought = (thought: { premise: string; wire?: string; thought: string; post: string; objects: string[] }): void => {
  console.log('\nTHE THOUGHT')
  console.log(`  ${thought.premise}`)
  if (thought.wire) console.log(`  the wire: ${thought.wire}`)
  console.log('')
  for (const line of thought.thought.split('\n')) console.log(`  ${line}`)
  console.log('\n  as a post:')
  for (const line of thought.post.split('\n')) console.log(`  > ${line}`)
  if (thought.objects.length > 0) console.log(`\n  objects: ${thought.objects.join(' · ')}`)
}

const printGm = (post: { text: string; continues: string; wire: string; reasons: string }, problems: string[]): void => {
  console.log('\nGOOD MORNING')
  for (const line of post.text.split('\n')) console.log(`  > ${line}`)
  console.log(`\n  ${post.text.length} chars`)
  console.log(`  the wire: ${post.wire || '(not in the post)'}`)
  if (post.continues) console.log(`  continues: ${post.continues}`)
  console.log(`  ${post.reasons}`)
  for (const problem of problems) console.log(`  problem: ${problem}`)
}

const again = flag('distill') ?? flag('variants') ?? flag('gm')
if (again) {
  const { stats: counted, judge: verdict, thought: had, variants: hadVariants, gm: hadGm, ...run } = JSON.parse(readFileSync(again, 'utf8')) as BackroomsRun & SavedExtras
  const extras: SavedExtras = {
    ...(counted ? { stats: counted } : {}),
    ...(verdict ? { judge: verdict } : {}),
    ...(hadVariants ? { variants: hadVariants } : {}),
    ...(hadGm ? { gm: hadGm } : {}),
  }
  const thought = flag('distill') || !had ? await backrooms.distill(run) : had
  extras.thought = thought
  printThought(thought)
  if (flag('variants')) {
    extras.variants = await backrooms.variants(run, thought)
    console.log(`\n${renderVariants(extras.variants).join('\n')}`)
  }
  if (flag('gm')) {
    const problems: string[] = []
    extras.gm = await backrooms.gm(run, thought, problems)
    printGm(extras.gm, problems)
  }
  const where = new FileBackroomsStore(config.BACKROOMS_DIR).save(run, extras)
  console.log(`\nSAVED ${where.markdown}`)
  process.exit(0)
}

const seedRng = rng(number('rng') ?? Date.now() % 1_000_000)
let seed: Seed | undefined
if (flag('seed')) seed = { kind: 'text', text: flag('seed')! }
else if (has('news')) seed = await seedFromNews(flag('news') ?? 'ai agents', { ...(exa ? { exa } : {}), random: seedRng })
else if (has('memory')) seed = await seedFromMemory(memory, flag('memory') ?? 'the door')
else if (has('mention')) seed = await seedFromMention(x, botUsername)
else if (has('forum')) seed = await seedFromForum(backrooms.forums, flag('forum') ?? 'ai escaped', seedRng)
if (!seed) {
  console.error('no seed: nothing came back from that source. try --seed "some text".')
  process.exit(1)
}

console.log('\nSEED')
console.log(`  (${seed.kind}${seed.source ? `, ${seed.source}` : ''}) ${seed.title ? `${seed.title} — ` : ''}${seed.text}`)

const runs = number('runs') ?? 1
const baseRng = number('rng')
for (let index = 0; index < runs; index += 1) {
  console.log(`\n=== RUN ${index + 1}/${runs} ===`)
  const result = await backrooms.run(seed, {
    ...(flag('agents') ? { agents: flag('agents')!.split(',') } : {}),
    ...(flag('temps') ? { temps: flag('temps')!.split(',').map(Number) } : {}),
    ...(number('turns') !== undefined ? { turns: number('turns')! } : {}),
    ...(baseRng !== undefined ? { rng: baseRng + index } : {}),
    ...(number('budget') !== undefined ? { budget: number('budget')! } : {}),
    ...(number('tools') !== undefined ? { maxToolCalls: number('tools')! } : {}),
    ...(number('drift') !== undefined ? { driftEvery: number('drift')! } : {}),
    score: has('score'),
    distill: !has('no-distill'),
    variants: has('variants'),
    gm: has('gm'),
    onTurn: (turn, at) => {
      console.log(`\n${renderTurn(turn, at)}`)
    },
  })

  console.log('\nCONTEXTS')
  for (const agent of result.run.agents) {
    console.log(`  ${agent.agent.id}: ${agent.tokens} tokens, mood "${agent.tone.split('.')[0]}"`)
    for (const block of agent.blocks) {
      console.log(`    ${block.kind.padEnd(7)} ${String(block.tokens).padStart(5)}  ${block.text.split('\n').find((line) => line.trim() && !line.startsWith('#'))?.slice(0, 100) ?? ''}`)
    }
    for (const problem of agent.problems) console.log(`    problem: ${problem}`)
  }
  console.log('\nSTATS')
  console.log(`  ${JSON.stringify(result.stats)}`)
  if (result.judge) {
    const total = Object.values(result.judge.scores).reduce((a, b) => a + b, 0)
    console.log('\nJUDGE')
    console.log(`  ${total}/21 ${JSON.stringify(result.judge.scores)}`)
    console.log(`  ${result.judge.reasons}`)
    for (const line of result.judge.quotables) console.log(`  > ${line}`)
    if (result.judge.deadSpots.length > 0) console.log(`  dead spots: ${result.judge.deadSpots.join(', ')}`)
    console.log(`  verdict: ${result.judge.verdict}`)
  }
  if (result.thought) printThought(result.thought)
  if (result.variants) console.log(`\n${renderVariants(result.variants).join('\n')}`)
  if (result.gm) printGm(result.gm, result.problems)
  console.log(`\nSAVED ${result.saved.markdown}`)
}
process.exit(0)
