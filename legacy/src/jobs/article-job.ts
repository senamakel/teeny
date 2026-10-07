import type { ArticleResult } from '../article/pipeline.js'
import { renderRoom } from '../article/pipeline.js'
import { renderStructure } from '../article/stages.js'
import type { ArticleStore } from '../article/store.js'
import type { BackroomsRun } from '../backrooms/types.js'
import type { Cadence } from '../cadence/cadence.js'
import type { Journal } from '../journal/journal.js'

export interface ArticleJobDependencies {
  pipeline: { run(): Promise<ArticleResult> }
  articles: Pick<ArticleStore, 'latestPublishedAt'>
  logger: { info: (object: unknown, message?: string) => void; error: (object: unknown, message?: string) => void }
  /** Refuses to run again inside this window, in memory and against the last published article on disk. */
  minIntervalMs?: number
  now?: () => number
  journal?: Journal
  cadence?: Pick<Cadence, 'claim' | 'within'>
  /** The room's transcript for the journal, when the run is on disk. */
  readRun?: (path: string) => Promise<BackroomsRun | undefined>
}

/**
 * One article per scheduled run, with the serial's guards and one more:
 * the last published article's time is read from the store, so a restart
 * near the cron minute cannot publish twice even though `lastRunAt` is
 * in memory. A rehearsal (publish off) is not held to that window; it
 * writes a record and nothing else.
 */
export class ArticleJob {
  private lastRunAt = 0

  constructor(private readonly dependencies: ArticleJobDependencies) {}

  async run(options: { force?: boolean } = {}): Promise<ArticleResult | undefined> {
    const { dependencies } = this
    const now = (dependencies.now ?? Date.now)()
    const minimum = dependencies.minIntervalMs ?? 48 * 60 * 60 * 1_000
    if (!options.force && this.lastRunAt > 0 && now - this.lastRunAt < minimum) {
      dependencies.logger.info({ sinceLastRunMs: now - this.lastRunAt, minimum }, 'article job: skipping, it already ran recently')
      return undefined
    }
    const published = await dependencies.articles.latestPublishedAt().catch(() => undefined)
    if (!options.force && published && now - Date.parse(published) < minimum) {
      dependencies.logger.info({ publishedAt: published, minimum }, 'article job: skipping, an article was published recently')
      return undefined
    }
    const claim = await dependencies.cadence?.claim('article')
    if (claim && !claim.ok && !options.force) {
      dependencies.logger.info(
        { ...claim },
        `article job: skipping, the cadence says ${claim.reason === 'gap' ? 'too soon after the last post' : 'the timeline is held for a hot post'}`,
      )
      return undefined
    }
    this.lastRunAt = now

    let result: ArticleResult
    try {
      const pipeline = dependencies.pipeline
      result = dependencies.cadence ? await dependencies.cadence.within('article', () => pipeline.run()) : await pipeline.run()
    } catch (error) {
      await this.journalFailure(now, error)
      throw error
    }
    dependencies.logger.info(
      {
        outcome: result.outcome,
        slug: result.slug,
        title: result.structure?.title,
        engine: result.engine,
        words: result.wordCount,
        images: result.images.length + (result.cover ? 1 : 0),
        problems: result.problems,
        postId: result.postId,
      },
      'article job: done',
    )
    await this.journal(now, result)
    return result
  }

  private id(now: number): string {
    return `article:${new Date(now).toISOString().slice(0, 10)}`
  }

  private async journal(now: number, result: ArticleResult): Promise<void> {
    const { journal, logger, readRun } = this.dependencies
    if (!journal) return
    const chosen = result.rooms.find((room) => room.saved && room.runId && !room.rejected)
    const run = chosen?.saved && readRun ? await readRun(chosen.saved).catch(() => undefined) : undefined
    try {
      await journal.record({
        id: this.id(now),
        at: new Date(now).toISOString(),
        trigger: 'article',
        incoming: `an article: ${result.rooms.length} room(s)${result.engine ? ` about ${result.engine.split(':')[0]}` : ''}${result.seed?.title ? `, seeded by "${result.seed.title}"` : ''}`,
        tone: 'neutral',
        recalled: result.thought ? [result.thought.thought] : [],
        tools: [
          ...(result.cover ? [{ name: 'generate_image', result: result.cover.id }] : []),
          ...result.images.map((image) => ({ name: 'generate_image', result: image.media.id })),
          ...(result.postId ? [{ name: 'article_publish', result: result.postId }] : []),
        ],
        thinking: [
          ...result.rooms.map((room) => `room ${room.index + 1}${room.runId ? ` ${room.runId}` : ''}: ${room.rejected ?? `scored ${room.total ?? '?'}`}`),
          ...(result.thought ? ['', `premise: ${result.thought.premise}`] : []),
          ...(result.structure ? ['', 'the plan:', renderStructure(result.structure)] : []),
          ...(result.structureCheck ? ['', `the editor: ${result.structureCheck.ok ? 'ok' : result.structureCheck.problems.join('; ')}`] : []),
          ...(result.proof
            ? ['', `the proof-reader: ${result.proof.review.verdict}, ${result.proof.review.findings.length} finding(s), fix ${result.proof.applied ? 'applied' : 'not applied'}`]
            : []),
          ...(run ? ['', 'the room:', renderRoom(run)] : []),
        ].join('\n'),
        said: result.structure ? `${result.structure.title}\n${result.structure.dek}` : '',
        declined: result.problems,
        outcome: result.outcome === 'structure' ? 'dry-run' : result.outcome,
        ...(result.outcome === 'skipped' ? { outcomeNote: result.problems.at(-1) ?? 'nothing to publish' } : {}),
        ...(result.outcome === 'held'
          ? { outcomeNote: result.problems.find((problem) => problem.startsWith('content filter withheld')) ?? 'content filter withheld the article' }
          : {}),
        ...(result.outcome === 'dry-run' || result.outcome === 'structure' ? { outcomeNote: `rehearsal saved as ${result.slug}` } : {}),
      })
    } catch (error) {
      logger.error({ err: error }, 'article job: could not journal the run')
    }
  }

  private async journalFailure(now: number, error: unknown): Promise<void> {
    const { journal, logger } = this.dependencies
    if (!journal) return
    try {
      await journal.record({
        id: this.id(now),
        at: new Date(now).toISOString(),
        trigger: 'article',
        incoming: 'an article',
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
      logger.error({ err: journalError }, 'article job: could not journal the failure')
    }
  }
}
