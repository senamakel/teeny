import type { Backrooms, RunResult } from '../backrooms/wiring.js'
import type { LorePost } from '../backrooms/lore.js'
import type { Thought } from '../backrooms/distill.js'
import type { Seed } from '../backrooms/types.js'
import type { SafetyGate } from '../safety/gate.js'
import type { Memory, XTools } from '../types.js'

/** Where the day's seed comes from, tried in the order given until one answers. */
export type SeedSource = 'news' | 'forum' | 'memory'

export interface LorePipelineOptions {
  backrooms: Pick<Backrooms, 'run' | 'lore'>
  /** The seed finders, by source; a source with no finder is skipped. */
  seeds: Partial<Record<SeedSource, (topic: string) => Promise<Seed | undefined>>>
  /** The order the sources are tried in today; the first that answers wins. */
  sources: SeedSource[]
  /** What the wire is asked for: one is drawn a day. */
  topics: string[]
  x: Pick<XTools, 'createPost'>
  memory: Pick<Memory, 'captureActivity'>
  /** How many turns the room runs. Absent, the backrooms' own setting. */
  turns?: number
  dryRun: boolean
  /** Off, nothing is posted even when dryRun is off. The rehearsal script's guard. */
  allowPosting?: boolean
  random?: () => number
  now?: () => number
  logger?: { info: (object: unknown, message?: string) => void }
  safety?: Pick<SafetyGate, 'outbound'>
}

export interface LoreResult {
  seed?: Seed
  /** Which source and topic the seed came from. */
  from?: { source: SeedSource; topic: string }
  room?: { id: string; thought: Thought; saved: string }
  post?: LorePost
  text: string
  postedId?: string
  outcome: 'posted' | 'dry-run' | 'skipped' | 'held'
  problems: string[]
}

/**
 * The afternoon lore post: a page from the lab, five days a week.
 *
 * Something comes through the wire -- a story, a forum post, a recall --
 * the room runs on it inside the lore, the editor keeps one thought, and
 * the thought is written up as a post with the soul, the lore and the last
 * posts around it. Nothing in it draws or films: the serial's episodes and
 * the morning post spend the media budget; this is the lab told in words.
 *
 * Every stage can decline and the day is then quiet with a reason: no
 * seed (every source dry), no thought (the room drifted to nothing), a
 * post the filter holds. A quiet day is journaled; a wrong post is not
 * recoverable.
 */
export class LorePipeline {
  constructor(private readonly options: LorePipelineOptions) {}

  async run(): Promise<LoreResult> {
    const { options } = this
    const now = (options.now ?? Date.now)()
    const random = options.random ?? Math.random
    const problems: string[] = []
    const log = (event: string, detail: Record<string, unknown> = {}): void =>
      options.logger?.info(detail, `lore: ${event}`)

    // 1. The seed. One topic drawn for the day, the sources in today's order.
    const topic = options.topics[Math.floor(random() * options.topics.length)] ?? 'ai agents'
    let seed: Seed | undefined
    let from: LoreResult['from']
    for (const source of options.sources) {
      const find = options.seeds[source]
      if (!find) continue
      try {
        seed = await find(topic)
      } catch (error) {
        problems.push(`${source} could not be read: ${messageOf(error)}`)
        continue
      }
      if (seed) {
        from = { source, topic }
        break
      }
      problems.push(`${source} had nothing on "${topic}"`)
    }
    if (!seed || !from) {
      log('no seed', { topic, problems })
      return { text: '', outcome: 'skipped', problems: [...problems, 'nothing came through the wire; no room was opened'] }
    }
    log('seed', { source: from.source, topic, kind: seed.kind, title: seed.title })

    // 2. The room, and the thought it leaves.
    let result: RunResult
    try {
      result = await options.backrooms.run(seed, options.turns !== undefined ? { turns: options.turns } : {})
    } catch (error) {
      problems.push(`the room failed: ${messageOf(error)}`)
      return { seed, from, text: '', outcome: 'skipped', problems }
    }
    problems.push(...result.problems.map((problem) => `the room: ${problem}`))
    if (!result.thought) {
      problems.push('the room ran but left no thought')
      return { seed, from, text: '', outcome: 'skipped', problems }
    }
    const room = { id: result.run.id, thought: result.thought, saved: result.saved.markdown }
    log('thought', { room: room.id, premise: room.thought.premise })

    // 3. The post, from the thought.
    let post: LorePost
    try {
      post = await options.backrooms.lore(result.run, result.thought, problems)
    } catch (error) {
      problems.push(`the thought could not be written up: ${messageOf(error)}`)
      return { seed, from, room, text: '', outcome: 'skipped', problems }
    }
    let text = post.text
    const outgoing = await options.safety?.outbound(text, { surface: 'lore', kind: 'draft', tool: 'lore_post' })
    if (outgoing) {
      if (outgoing.fixes.length > 0) problems.push(`the content filter edited the post: ${outgoing.fixes.join('; ')}`)
      if (!outgoing.decision.ok) {
        log('post held', { category: outgoing.decision.category, by: outgoing.decision.by, reason: outgoing.decision.reason })
        problems.push(`content filter withheld the post: ${outgoing.decision.category}`)
        return { seed, from, room, post, text: '', outcome: 'held', problems }
      }
      text = outgoing.text
    }

    // 4. Post, or say what would have been posted.
    const base: LoreResult = { seed, from, room, post, text, outcome: 'dry-run', problems }
    if (options.dryRun || options.allowPosting === false) {
      await this.capture(now, base)
      return base
    }
    const posted = await options.x.createPost(text)
    log('posted', { id: posted.id, room: room.id })
    const done: LoreResult = { ...base, outcome: 'posted', postedId: posted.id }
    await this.capture(now, done)
    return done
  }

  /** What went out, with the room's premise, which is what the next run reads so a page is not told twice. */
  private async capture(now: number, result: LoreResult): Promise<void> {
    await this.options.memory
      .captureActivity('lore_post', result.postedId ?? `dry-run:${now}`, {
        text: result.text,
        premise: result.room?.thought.premise,
        objects: result.room?.thought.objects,
        seed: result.seed ? { kind: result.seed.kind, title: result.seed.title, source: result.seed.source } : undefined,
        room: result.room?.id,
        published: result.outcome === 'posted',
      })
      .catch((error: unknown) => {
        result.problems.push(`could not remember the post: ${messageOf(error)}`)
      })
  }
}

function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
