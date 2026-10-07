import type { Journal } from '../journal/journal.js'
import type { Cadence } from '../cadence/cadence.js'
import type { QuoteResult } from '../quote/pipeline.js'

export interface QuoteJobDependencies {
  pipeline: { run(): Promise<QuoteResult> }
  logger: { info: (object: unknown, message?: string) => void; error: (object: unknown, message?: string) => void }
  /** Refuses to run again inside this window. Defaults to 20 hours. */
  minIntervalMs?: number
  now?: () => number
  journal?: Journal
  /** The content calendar's spacing rule; absent, the job trusts its own timer. */
  cadence?: Pick<Cadence, 'claim' | 'within'>
}

/**
 * One quote post a day.
 *
 * The same interval guard as the other posting jobs, for the same reason:
 * a container restarted near the cron minute would run again, and the
 * pipeline's own log only stops it quoting the same post, not quoting a
 * second one an hour after the first.
 */
export class QuoteJob {
  private lastRunAt = 0

  constructor(private readonly dependencies: QuoteJobDependencies) {}

  /** Returns the result, or undefined when it declined to run, which is not an error. */
  async run(options: { force?: boolean } = {}): Promise<QuoteResult | undefined> {
    const now = (this.dependencies.now ?? Date.now)()
    const minimum = this.dependencies.minIntervalMs ?? 20 * 60 * 60 * 1_000
    if (!options.force && this.lastRunAt > 0 && now - this.lastRunAt < minimum) {
      this.dependencies.logger.info({ sinceLastRunMs: now - this.lastRunAt, minimum }, 'quote job: skipping, it already ran recently')
      return undefined
    }
    const claim = await this.dependencies.cadence?.claim('quote')
    if (claim && !claim.ok && !options.force) {
      this.dependencies.logger.info({ ...claim }, `quote job: skipping, the cadence says ${claim.reason === 'gap' ? 'too soon after the last post' : 'the timeline is held for a hot post'}`)
      return undefined
    }
    this.lastRunAt = now

    let result: QuoteResult
    try {
      const pipeline = this.dependencies.pipeline
      result = this.dependencies.cadence
        ? await this.dependencies.cadence.within('quote', () => pipeline.run())
        : await pipeline.run()
    } catch (error) {
      await this.journalFailure(now, error)
      throw error
    }
    this.dependencies.logger.info(
      {
        outcome: result.outcome,
        quoted: result.chosen?.id,
        author: result.chosen?.authorUsername,
        candidates: result.candidates.length,
        problems: result.problems,
        postedId: result.postedId,
      },
      'quote job: done',
    )
    await this.journal(now, result)
    return result
  }

  private id(now: number): string {
    return `quote:${new Date(now).toISOString().slice(0, 10)}`
  }

  private async journal(now: number, result: QuoteResult): Promise<void> {
    const { journal, logger } = this.dependencies
    if (!journal) return
    try {
      await journal.record({
        id: this.id(now),
        at: new Date(now).toISOString(),
        trigger: 'quote',
        incoming: result.chosen
          ? `@${result.chosen.authorUsername ?? 'someone'}: ${result.chosen.text.replace(/\s+/g, ' ').trim()}`
          : `${result.searched} posts read, ${result.candidates.length} candidates`,
        tone: 'friendly',
        recalled: [result.recalled].filter(Boolean),
        tools: [
          { name: 'search', result: `${result.searched} posts, ${result.candidates.length} kept, ${result.skipped.length} skipped` },
        ],
        thinking: [
          result.why,
          ...(result.skipped.length > 0
            ? [`skipped: ${summarise(result.skipped.map((entry) => entry.reason))}`]
            : []),
        ].join('\n'),
        said: result.outcome === 'held' ? '' : result.text,
        declined: result.problems,
        outcome: result.outcome,
        ...(result.outcome === 'held'
          ? { outcomeNote: result.problems.find((problem) => problem.startsWith('content filter withheld')) ?? 'content filter withheld the text' }
          : result.outcome === 'skipped'
            ? { outcomeNote: result.why }
            : {}),
      })
    } catch (error) {
      logger.error({ err: error }, 'quote job: could not journal the turn')
    }
  }

  private async journalFailure(now: number, error: unknown): Promise<void> {
    const { journal, logger } = this.dependencies
    if (!journal) return
    try {
      await journal.record({
        id: this.id(now),
        at: new Date(now).toISOString(),
        trigger: 'quote',
        incoming: 'the quote post',
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
      logger.error({ err: journalError }, 'quote job: could not journal the failure')
    }
  }
}

/** "already quoted ×3, names another token ×2": the reasons, counted. */
function summarise(reasons: string[]): string {
  const counts = new Map<string, number>()
  for (const reason of reasons) {
    const key = reason.replace(/@\S+ was quoted on \S+/, 'author quoted lately').replace(/\(\$\w+\)/, '')
    counts.set(key.trim(), (counts.get(key.trim()) ?? 0) + 1)
  }
  return [...counts.entries()].map(([reason, count]) => (count > 1 ? `${reason} ×${count}` : reason)).join(', ')
}
