import type { Cadence } from '../cadence/cadence.js'
import type { Journal } from '../journal/journal.js'
import type { NarrativeResult } from '../narrative/pipeline.js'
import { renderTranscript } from '../narrative/stages.js'

export interface NarrativeJobDependencies {
  pipeline: { run(): Promise<NarrativeResult> }
  logger: { info: (object: unknown, message?: string) => void; error: (object: unknown, message?: string) => void }
  /** Refuses to run again inside this window. Defaults to 20 hours. */
  minIntervalMs?: number
  now?: () => number
  journal?: Journal
  cadence?: Pick<Cadence, 'claim' | 'within'>
}

/**
 * One episode of the serial per scheduled run, with the morning job's
 * guards: an interval so a restart near the cron minute does not post two
 * episodes, and the cadence so it keeps its distance from the last post.
 */
export class NarrativeJob {
  private lastRunAt = 0

  constructor(private readonly dependencies: NarrativeJobDependencies) {}

  async run(options: { force?: boolean } = {}): Promise<NarrativeResult | undefined> {
    const now = (this.dependencies.now ?? Date.now)()
    const minimum = this.dependencies.minIntervalMs ?? 20 * 60 * 60 * 1_000
    if (!options.force && this.lastRunAt > 0 && now - this.lastRunAt < minimum) {
      this.dependencies.logger.info({ sinceLastRunMs: now - this.lastRunAt, minimum }, 'narrative job: skipping, it already ran recently')
      return undefined
    }
    const claim = await this.dependencies.cadence?.claim('narrative')
    if (claim && !claim.ok && !options.force) {
      this.dependencies.logger.info(
        { ...claim },
        `narrative job: skipping, the cadence says ${claim.reason === 'gap' ? 'too soon after the last post' : 'the timeline is held for a hot post'}`,
      )
      return undefined
    }
    this.lastRunAt = now

    let result: NarrativeResult
    try {
      const pipeline = this.dependencies.pipeline
      result = this.dependencies.cadence
        ? await this.dependencies.cadence.within('narrative', () => pipeline.run())
        : await pipeline.run()
    } catch (error) {
      await this.journalFailure(now, error)
      throw error
    }
    this.dependencies.logger.info(
      {
        outcome: result.outcome,
        episode: result.episodeNumber,
        title: result.chosen?.episode?.title,
        attached: result.attached,
        problems: result.problems,
        postedId: result.postedId,
      },
      'narrative job: done',
    )
    await this.journal(now, result)
    return result
  }

  private id(now: number): string {
    return `narrative:${new Date(now).toISOString().slice(0, 10)}`
  }

  private async journal(now: number, result: NarrativeResult): Promise<void> {
    const { journal, logger } = this.dependencies
    if (!journal) return
    const episode = result.chosen?.episode
    try {
      await journal.record({
        id: this.id(now),
        at: new Date(now).toISOString(),
        trigger: 'narrative',
        incoming: `episode ${result.episodeNumber}: ${result.branches.length} rooms written, ${result.branches.filter((b) => b.episode && !b.rejected).length} cuts survived`,
        tone: 'neutral',
        recalled: [result.briefing],
        tools: [
          ...(result.image ? [{ name: 'generate_image', result: result.image.id }] : []),
          ...(result.video ? [{ name: 'generate_video', result: result.video.id }] : []),
        ],
        thinking: [
          ...(episode ? [`title: ${episode.title}`, `logline: ${episode.logline}`, `state after: ${episode.stateAfter}`] : []),
          ...(result.liveLore.length > 0 ? [`live lore: ${result.liveLore.join(', ')}`] : []),
          ...result.branches.map(
            (branch) =>
              `branch ${branch.index + 1}${branch.episode ? ` "${branch.episode.title}"` : ''}: ${branch.rejected ?? `composite ${branch.composite}`}`,
          ),
          ...(result.chosen ? ['', 'the room:', renderTranscript(result.chosen.transcript)] : []),
          ...(episode ? ['', `shot list: ${episode.shotList}`] : []),
        ].join('\n'),
        said: result.outcome === 'held' ? '' : result.caption,
        declined: result.problems,
        outcome: result.outcome,
        ...(result.outcome === 'skipped' ? { outcomeNote: 'nothing could be attached' } : {}),
        ...(result.outcome === 'held'
          ? { outcomeNote: result.problems.find((problem) => problem.startsWith('content filter withheld')) ?? 'content filter withheld the caption' }
          : {}),
      })
    } catch (error) {
      logger.error({ err: error }, 'narrative job: could not journal the turn')
    }
  }

  private async journalFailure(now: number, error: unknown): Promise<void> {
    const { journal, logger } = this.dependencies
    if (!journal) return
    try {
      await journal.record({
        id: this.id(now),
        at: new Date(now).toISOString(),
        trigger: 'narrative',
        incoming: 'an episode of the serial',
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
      logger.error({ err: journalError }, 'narrative job: could not journal the failure')
    }
  }
}
