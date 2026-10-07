import type { Journal } from '../journal/journal.js'
import type { Cadence } from '../cadence/cadence.js'
import { rotaSlot, type Rota } from './cycle.js'
import type { LoreResult } from '../lore/pipeline.js'

export interface LoreJobDependencies {
  pipeline: { run(): Promise<LoreResult> }
  logger: { info: (object: unknown, message?: string) => void; error: (object: unknown, message?: string) => void }
  /** Refuses to run again inside this window. Defaults to 20 hours. */
  minIntervalMs?: number
  now?: () => number
  journal?: Journal
  /** The posting rota and the slots that are this job's. Absent, it posts every day the cron fires. */
  rota?: { rota: Rota; slots: number[] }
  cadence?: Pick<Cadence, 'claim' | 'within'>
  /** Which lore job this is, when there are two: the journal id and the cadence kind carry it. `lore` by default. */
  name?: string
}

/**
 * The lore post, on its days.
 *
 * The chart and the moon chart have one slot each on the rota; this job has
 * the rest, so the afternoon slot is filled every day and by one thing. The
 * same interval guard as the others, for the same reason.
 */
export class LoreJob {
  private lastRunAt = 0

  constructor(private readonly dependencies: LoreJobDependencies) {}

  /** Returns the result, or undefined when it declined to run, which is not an error. */
  async run(options: { force?: boolean } = {}): Promise<LoreResult | undefined> {
    const now = (this.dependencies.now ?? Date.now)()
    const { rota } = this.dependencies
    if (!options.force && rota && !rota.slots.includes(rotaSlot(rota.rota, now))) {
      this.dependencies.logger.info({ slot: rotaSlot(rota.rota, now), slots: rota.slots, rota: rota.rota }, 'lore job: skipping, not its day on the rota')
      return undefined
    }
    const minimum = this.dependencies.minIntervalMs ?? 20 * 60 * 60 * 1_000
    if (!options.force && this.lastRunAt > 0 && now - this.lastRunAt < minimum) {
      this.dependencies.logger.info({ sinceLastRunMs: now - this.lastRunAt, minimum }, 'lore job: skipping, it already ran recently')
      return undefined
    }
    const claim = await this.dependencies.cadence?.claim(this.name)
    if (claim && !claim.ok && !options.force) {
      this.dependencies.logger.info({ ...claim }, `lore job: skipping, the cadence says ${claim.reason === 'gap' ? 'too soon after the last post' : 'the timeline is held for a hot post'}`)
      return undefined
    }
    this.lastRunAt = now

    let result: LoreResult
    try {
      const pipeline = this.dependencies.pipeline
      result = this.dependencies.cadence
        ? await this.dependencies.cadence.within(this.name, () => pipeline.run())
        : await pipeline.run()
    } catch (error) {
      await this.journalFailure(now, error)
      throw error
    }
    this.dependencies.logger.info(
      { outcome: result.outcome, seed: result.from, room: result.room?.id, problems: result.problems, postedId: result.postedId },
      'lore job: done',
    )
    await this.journal(now, result)
    return result
  }

  private get name(): string {
    return this.dependencies.name ?? 'lore'
  }

  private id(now: number): string {
    return `${this.name}:${new Date(now).toISOString().slice(0, 10)}`
  }

  private async journal(now: number, result: LoreResult): Promise<void> {
    const { journal, logger } = this.dependencies
    if (!journal) return
    try {
      await journal.record({
        id: this.id(now),
        at: new Date(now).toISOString(),
        trigger: 'lore',
        incoming: result.seed
          ? `${result.seed.kind}${result.seed.source ? ` (${result.seed.source})` : ''}: ${result.seed.title ?? result.seed.text.slice(0, 200)}`
          : 'nothing came through the wire',
        tone: 'neutral',
        recalled: [],
        tools: result.room ? [{ name: 'backrooms', result: `${result.room.id}: ${result.room.thought.premise}` }] : [],
        thinking: [
          result.room ? result.room.thought.thought : undefined,
          result.post?.reasons,
          result.post?.continues ? `continues: ${result.post.continues}` : undefined,
        ]
          .filter(Boolean)
          .join('\n'),
        said: result.outcome === 'held' ? '' : result.text,
        declined: result.problems,
        outcome: result.outcome,
        ...(result.outcome === 'held'
          ? { outcomeNote: result.problems.find((problem) => problem.startsWith('content filter withheld')) ?? 'content filter withheld the post' }
          : result.outcome === 'skipped'
            ? { outcomeNote: result.problems.at(-1) ?? 'nothing to post' }
            : {}),
      })
    } catch (error) {
      logger.error({ err: error }, 'lore job: could not journal the turn')
    }
  }

  private async journalFailure(now: number, error: unknown): Promise<void> {
    const { journal, logger } = this.dependencies
    if (!journal) return
    try {
      await journal.record({
        id: this.id(now),
        at: new Date(now).toISOString(),
        trigger: 'lore',
        incoming: 'the lore post',
        tone: 'neutral',
        recalled: [],
        tools: [],
        thinking: '',
        said: '',
        declined: [],
        outcome: 'failed',
        outcomeNote: error instanceof Error ? error.message : String(error),
      })
    } catch (journalError) {
      logger.error({ err: journalError }, 'lore job: could not journal the failure')
    }
  }
}
