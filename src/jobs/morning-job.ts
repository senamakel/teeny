import type { Journal } from '../journal/journal.js'
import type { MorningResult } from '../morning/pipeline.js'

/**
 * The calendar item the morning post stands in for.
 *
 * calendar.json ships a "gm" that draws Tiny somewhere pleasant at seven;
 * the morning job is the same slot done the long way, around the news, an
 * hour later. Both on would say good morning twice before breakfast, which
 * is the repetitive-publishing pattern, so the service gives the slot to the
 * morning job and skips the calendar item of this name while it runs.
 */
export const MORNING_CALENDAR_SLOT = 'gm'

export interface MorningJobDependencies {
  pipeline: { run(): Promise<MorningResult> }
  logger: { info: (object: unknown, message?: string) => void; error: (object: unknown, message?: string) => void }
  /** Refuses to run again inside this window. Defaults to 20 hours. */
  minIntervalMs?: number
  now?: () => number
  journal?: Journal
}

/**
 * One good-morning post a day.
 *
 * The same interval guard as the chart job, for the same reason: `node-cron`
 * fires on whatever process is alive, and a container restarted near the
 * cron minute would say good morning twice. Two morning posts an hour apart
 * is the repetitive-publishing pattern that cost this account its reach
 * once, so the job refuses rather than trusting the timer.
 */
export class MorningJob {
  private lastRunAt = 0

  constructor(private readonly dependencies: MorningJobDependencies) {}

  /** Returns the result, or undefined when it declined to run, which is not an error. */
  async run(options: { force?: boolean } = {}): Promise<MorningResult | undefined> {
    const now = (this.dependencies.now ?? Date.now)()
    const minimum = this.dependencies.minIntervalMs ?? 20 * 60 * 60 * 1_000
    if (!options.force && this.lastRunAt > 0 && now - this.lastRunAt < minimum) {
      this.dependencies.logger.info(
        { sinceLastRunMs: now - this.lastRunAt, minimum },
        'morning job: skipping, it already ran recently',
      )
      return undefined
    }
    this.lastRunAt = now

    let result: MorningResult
    try {
      result = await this.dependencies.pipeline.run()
    } catch (error) {
      await this.journalFailure(now, error)
      throw error
    }
    this.dependencies.logger.info(
      {
        outcome: result.outcome,
        attached: result.attached,
        story: result.pick.headline,
        problems: result.problems,
        postedId: result.postedId,
      },
      'morning job: done',
    )
    await this.journal(now, result)
    return result
  }

  private id(now: number): string {
    return `morning:${new Date(now).toISOString().slice(0, 10)}`
  }

  /** The morning turn, for the mind page: the caption is what went out, the rest is the thinking. */
  private async journal(now: number, result: MorningResult): Promise<void> {
    const { journal, logger } = this.dependencies
    if (!journal) return
    try {
      await journal.record({
        id: this.id(now),
        at: new Date(now).toISOString(),
        trigger: 'morning',
        incoming: `${result.stories.length} stories read, ${result.droppedAsPolitical} dropped as political`,
        tone: 'neutral',
        recalled: [result.recalled.history, result.recalled.community].filter(Boolean),
        tools: [
          { name: 'news', result: result.story ? `${result.story.title} (${result.story.source})` : 'no story chosen' },
          ...(result.image ? [{ name: 'generate_image', result: result.image.id }] : []),
          ...(result.video ? [{ name: 'generate_video', result: result.video.id }] : []),
        ],
        thinking: [
          `story: ${result.pick.headline}`,
          `angle: ${result.pick.angle}`,
          ...(result.seed ? [`seed: ${result.seed.url}`] : []),
          ...result.scene.concepts.map((concept, index) => `concept ${index + 1}: ${concept}`),
          `scene: ${result.scene.scene}`,
          `cue: ${result.scene.storyCue}`,
          ...(result.script ? [`script: ${result.script.script}`] : []),
        ].join('\n'),
        // A held caption is in the hold log for review, never on the public page.
        said: result.outcome === 'held' ? '' : result.caption,
        declined: result.problems,
        outcome: result.outcome,
        ...(result.outcome === 'skipped' ? { outcomeNote: 'nothing could be attached' } : {}),
        ...(result.outcome === 'held'
          ? { outcomeNote: result.problems.find((problem) => problem.startsWith('content filter withheld')) ?? 'content filter withheld the caption' }
          : {}),
      })
    } catch (error) {
      logger.error({ err: error }, 'morning job: could not journal the turn')
    }
  }

  private async journalFailure(now: number, error: unknown): Promise<void> {
    const { journal, logger } = this.dependencies
    if (!journal) return
    try {
      await journal.record({
        id: this.id(now),
        at: new Date(now).toISOString(),
        trigger: 'morning',
        incoming: 'the morning post',
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
      logger.error({ err: journalError }, 'morning job: could not journal the failure')
    }
  }
}
