import { applyHouseStyle, countEmoji } from '../agent/house-style.js'
import { MAX_POST_LENGTH } from '../agent/tools/x.js'
import type { TextModel } from '../media/reference.js'
import { askJson } from '../morning/stages.js'
import type { SafetyGate } from '../safety/gate.js'
import type { Memory, XPost, XTools } from '../types.js'
import { findCandidates, type Candidate, type CandidateSearch, type Found } from './candidates.js'
import { quotePickSchema, quotePrompt } from './prompt.js'
import type { QuoteLog } from './store.js'

export interface QuotePipelineOptions {
  x: Pick<XTools, 'createPost'> & CandidateSearch
  text: TextModel
  memory: Pick<Memory, 'recall' | 'captureActivity'>
  log: QuoteLog
  persona: string
  botUsername: string
  trustedHandles: string[]
  ownTokenAddresses: string[]
  ownSymbol: string
  /** The operator's ticker query, X search syntax; reposts and own posts are cut by the pipeline. */
  query: string
  /** How far back "recent" reaches. */
  maxAgeMs: number
  /** How long after quoting somebody before they can be quoted again. */
  authorCooldownMs: number
  /** How many posts each search reads; X floors it at 10 and bills every one. */
  maxResults: number
  /** How many survivors the picker sees. */
  keep?: number
  maxEmoji: number
  dryRun: boolean
  /** Off, nothing is posted even when dryRun is off. The rehearsal script's guard. */
  allowPosting?: boolean
  now?: () => number
  logger?: { info: (object: unknown, message?: string) => void }
  /** The content filter: the lexicon over every candidate, and the full gate over the words that go out. */
  safety?: Pick<SafetyGate, 'outbound'> & Partial<Pick<SafetyGate, 'screen'>>
}

export interface QuoteResult {
  candidates: Candidate[]
  skipped: Found['skipped']
  searched: number
  /** The post chosen, when one was. */
  chosen?: XPost
  why: string
  text: string
  postedId?: string
  outcome: 'posted' | 'dry-run' | 'skipped' | 'held'
  problems: string[]
  recalled: string
}

/**
 * One quote post a day: somebody bullish, lifted onto the timeline with a
 * line over it.
 *
 * A fixed sequence like the moon chart's, because the judgement in it is
 * one call -- which post, and what to say -- and everything around that
 * call is a rule: two searches, the filter, the log of what was quoted,
 * the content gate, the post. The log is the part that makes the series
 * safe to run on a timer: a post is quoted once, ever, and an author rests
 * a week between quotes, so the account does not become a stranger's
 * megaphone because they post the ticker every hour.
 */
export class QuotePipeline {
  constructor(private readonly options: QuotePipelineOptions) {}

  async run(): Promise<QuoteResult> {
    const { options } = this
    const now = (options.now ?? Date.now)()
    const nowIso = new Date(now).toISOString()
    const today = nowIso.slice(0, 10)
    const problems: string[] = []
    const log = (event: string, detail: Record<string, unknown> = {}): void =>
      options.logger?.info(detail, `quote: ${event}`)

    // 1. What this series has said, and what it has already quoted.
    const [history, quoted] = await Promise.all([
      options.memory
        .recall(
          'Which posts by other people has this account quote-posted lately, and what did it say over each? ' +
            'The author, the gist of their post, and the words the account put over it.',
        )
        .catch(() => ''),
      options.log.list(),
    ])

    // 2. The candidates: two searches, the rules, the best few.
    const found = await findCandidates(options.x, {
      query: options.query,
      maxResults: options.maxResults,
      keep: options.keep ?? 20,
      botUsername: options.botUsername,
      trustedHandles: options.trustedHandles,
      ownTokenAddresses: options.ownTokenAddresses,
      ownSymbol: options.ownSymbol,
      quoted,
      maxAgeMs: options.maxAgeMs,
      authorCooldownMs: options.authorCooldownMs,
      ...(options.safety?.screen ? { screen: (text: string) => options.safety!.screen!(text, 'inbound') } : {}),
      now,
      ...(options.logger ? { logger: options.logger } : {}),
    })
    log('candidates found', {
      searched: found.searched,
      kept: found.candidates.length,
      skipped: found.skipped.length,
    })
    const base = { candidates: found.candidates, skipped: found.skipped, searched: found.searched, problems, recalled: history }
    if (found.candidates.length === 0) {
      return { ...base, why: 'nothing recent survived the rules', text: '', outcome: 'skipped' }
    }

    // 3. The choice and the words, one call.
    const ask = quotePrompt(
      { persona: options.persona, botUsername: options.botUsername, today, now: nowIso },
      { candidates: found.candidates, history, today, maxEmoji: options.maxEmoji, maxLength: MAX_POST_LENGTH },
    )
    let pick = await askJson(options.text, { ...ask, schema: quotePickSchema })
    const index = pick.pick ?? 0
    if (index < 1 || index > found.candidates.length) {
      log('nothing picked', { why: pick.why })
      return { ...base, why: pick.why, text: '', outcome: 'skipped' }
    }
    if (pick.text.length > MAX_POST_LENGTH || countEmoji(pick.text) > options.maxEmoji || !pick.text.trim()) {
      const retry = await askJson(options.text, {
        system: ask.system,
        prompt:
          `${ask.prompt}\n\nYou picked ${index} and wrote ${pick.text.length} characters with ${countEmoji(pick.text)} ` +
          `emoji:\n${pick.text}\n\nKeep the pick. Rewrite the text under ${MAX_POST_LENGTH} characters with at most ` +
          `${options.maxEmoji} emoji, and not empty.`,
        schema: quotePickSchema,
      })
      pick = { ...retry, pick: index }
    }
    const chosen = found.candidates[index - 1]!.post
    let text = applyHouseStyle(pick.text, { maxEmoji: options.maxEmoji, maxLength: MAX_POST_LENGTH }).text
    // The handle is on the quoted post already; in the text it reads as a tag.
    if (chosen.authorUsername) {
      text = text.replace(new RegExp(`@${chosen.authorUsername}\\b`, 'gi'), '').replace(/[ \t]{2,}/g, ' ').trim()
    }
    log('picked', { postId: chosen.id, author: chosen.authorUsername, why: pick.why })
    if (!text) {
      problems.push('the picker chose a post and wrote nothing over it')
      return { ...base, chosen, why: pick.why, text: '', outcome: 'skipped' }
    }

    // 4. The filter, on the words that go out.
    const outgoing = await options.safety?.outbound(text, {
      surface: 'quote',
      kind: 'draft',
      postId: chosen.id,
      ...(chosen.authorUsername ? { authorUsername: chosen.authorUsername } : {}),
    })
    if (outgoing) {
      if (outgoing.fixes.length > 0) problems.push(`the content filter edited the text: ${outgoing.fixes.join('; ')}`)
      if (!outgoing.decision.ok) {
        log('text held', { category: outgoing.decision.category, by: outgoing.decision.by, reason: outgoing.decision.reason })
        problems.push(`content filter withheld the text: ${outgoing.decision.category}`)
        return { ...base, chosen, why: pick.why, text: '', outcome: 'held' }
      }
      text = outgoing.text
    }

    // 5. Post, or say what would have been posted. The log is written on a
    // dry run too, so a rehearsal does not pick the same post twice; a real
    // run then quotes something else, which is what a rehearsal is for.
    const result: QuoteResult = { ...base, chosen, why: pick.why, text, outcome: 'dry-run' }
    if (options.dryRun || options.allowPosting === false) {
      await this.remember(now, result)
      return result
    }
    const posted = await options.x.createPost(text, { quotePostId: chosen.id })
    log('posted', { id: posted.id, quoted: chosen.id })
    const done: QuoteResult = { ...result, outcome: 'posted', postedId: posted.id }
    await this.remember(now, done)
    return done
  }

  /** The log first, since it is the rule; memory second, since it is the voice. */
  private async remember(now: number, result: QuoteResult): Promise<void> {
    const chosen = result.chosen
    if (!chosen) return
    try {
      await this.options.log.record({
        postId: chosen.id,
        ...(chosen.authorUsername ? { authorUsername: chosen.authorUsername } : {}),
        quotedAt: new Date(now).toISOString(),
        ...(result.postedId ? { ownPostId: result.postedId } : {}),
      })
    } catch (error) {
      result.problems.push(`could not log the quote: ${error instanceof Error ? error.message : String(error)}`)
    }
    await this.options.memory
      .captureActivity('quote_post', result.postedId ?? `dry-run:${now}`, {
        text: result.text,
        quotedPostId: chosen.id,
        quotedAuthor: chosen.authorUsername,
        quotedText: chosen.text,
        why: result.why,
        published: result.outcome === 'posted',
      })
      .catch((error: unknown) => {
        result.problems.push(`could not remember the post: ${error instanceof Error ? error.message : String(error)}`)
      })
  }
}
