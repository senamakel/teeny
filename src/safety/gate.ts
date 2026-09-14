/**
 * The gate every message passes through twice: once on the way in, once on
 * the way out.
 *
 * Inbound, a prohibited message is not answered. Not declined, not
 * deflected, not roasted: not processed. The drafter never sees it, nothing
 * about it is remembered, and the journal records that something was held
 * and why, without the text.
 *
 * Outbound, the account's own draft is held to a stricter reading (profanity
 * counts, and every link is checked against the allowlist), and the caller
 * gets back either clean text to publish or a reason not to. The out gate
 * exists because the in gate is not enough: a clean question can still get
 * a dirty answer, from a model with a bad day or a web page with a plan.
 *
 * Order of checks is cheapest first and each one is sufficient to stop the
 * turn: links are cut, the lexicon is read, and only then is the judge asked.
 */
import type { HoldContext, HoldLog } from './holds.js'
import type { Judge } from './judge.js'
import { DEFAULT_LINK_ALLOWLIST, filterLinks, parseAllowlist, type AllowlistEntry } from './links.js'
import { screenText, type Direction } from './screen.js'

export type SafetyDecision =
  | { ok: true }
  | {
      ok: false
      /** What it was held for: a lexicon category, the judge's category, or `unknown`. */
      category: string
      /** For the log and the journal. Never contains the text. */
      reason: string
      /** Which check held it. */
      by: 'screen' | 'judge' | 'links'
    }

export interface OutboundResult {
  decision: SafetyDecision
  /** The text with disallowed links cut out. Only meaningful when `decision.ok`. */
  text: string
  /** What was changed, for logging. Empty when the draft was already clean. */
  fixes: string[]
}

export interface SafetyGateOptions {
  /** Absent, only the lexicon and the allowlist run. Present, it is asked last. */
  judge?: Judge
  /** Added to the built-in allowlist, in the same `host[/path]` spelling. */
  extraAllowedLinks?: string[]
  /**
   * Where every hold is written for review, and where operator-approved
   * links are read from. Absent, holds are logged and forgotten.
   */
  holds?: HoldLog
  /** How long approved links are cached before the log is asked again. */
  approvedLinksTtlMs?: number
  logger?: SafetyLogger
}

/** The two lines the gate writes: a hold, and an edit. Pino-shaped. */
export interface SafetyLogger {
  info(detail: Record<string, unknown>, message?: string): void
  warn(detail: Record<string, unknown>, message?: string): void
}

export class SafetyGate {
  private readonly judge: Judge | undefined
  private readonly configured: AllowlistEntry[]
  private readonly holds: HoldLog | undefined
  private readonly ttl: number
  private readonly logger: SafetyLogger | undefined
  private approved: { entries: AllowlistEntry[]; at: number } = { entries: [], at: 0 }

  constructor(options: SafetyGateOptions = {}) {
    this.judge = options.judge
    this.configured = parseAllowlist([...DEFAULT_LINK_ALLOWLIST, ...(options.extraAllowedLinks ?? [])])
    this.holds = options.holds
    this.ttl = options.approvedLinksTtlMs ?? 60_000
    this.logger = options.logger
  }

  /**
   * The built-in list plus whatever an operator has approved since. Read
   * from the log on a short cache, so an approval taken in the API process
   * reaches the reply process without a restart. A log that cannot be read
   * leaves the approved set as it was, never empties it.
   */
  private async allowlist(): Promise<AllowlistEntry[]> {
    if (this.holds && Date.now() - this.approved.at > this.ttl) {
      try {
        this.approved = { entries: parseAllowlist(await this.holds.allowedLinks()), at: Date.now() }
      } catch (error) {
        this.logger?.warn({ err: error }, 'safety: could not read approved links; keeping the last set')
        this.approved.at = Date.now()
      }
    }
    return [...this.configured, ...this.approved.entries]
  }

  /** Forgets the cached approvals, so the next check reads the log. */
  refreshApprovedLinks(): void {
    this.approved.at = 0
  }

  /** Writes a hold for review. Never throws: the decision has already been made. */
  private async hold(entry: Parameters<HoldLog['record']>[0]): Promise<void> {
    if (!this.holds) return
    try {
      await this.holds.record(entry)
    } catch (error) {
      this.logger?.warn({ err: error }, 'safety: could not record a hold')
    }
  }

  /** The lexicon alone, synchronous. For scrubbing context lines cheaply. */
  screen(text: string, direction: Direction = 'inbound'): boolean {
    return screenText(text, direction) === undefined
  }

  /**
   * Whether a message from somebody else is something the account processes
   * at all. The context says where it came from, so a hold can be reviewed
   * and, if the reviewer disagrees, acted on.
   */
  async inbound(text: string, context?: HoldContext): Promise<SafetyDecision> {
    const decision = await this.inboundDecision(text)
    if (!decision.ok && context) {
      await this.hold({
        direction: 'inbound',
        kind: context.kind ?? (context.surface === 'task' ? 'task' : 'message'),
        context,
        category: decision.category,
        reason: decision.reason,
        by: decision.by,
        text,
      })
    }
    return decision
  }

  private async inboundDecision(text: string): Promise<SafetyDecision> {
    const match = screenText(text, 'inbound')
    if (match) {
      this.logger?.info({ category: match.category, term: match.term }, 'safety: inbound held by the screen')
      return { ok: false, category: match.category, reason: `contains "${match.term}"`, by: 'screen' }
    }
    return this.ask(text, 'inbound')
  }

  /**
   * The account's own words, checked before they leave. The text handed back
   * has its links filtered; the caller publishes that, not what it passed in.
   * A cut link is a hold of its own even when the rest goes out, because a
   * cut link is the filter saying no to a host, and that is the decision a
   * reviewer most often wants to overturn.
   */
  async outbound(text: string, context?: HoldContext): Promise<OutboundResult> {
    const fixes: string[] = []
    const links = filterLinks(text, await this.allowlist())
    if (links.removed.length > 0) {
      fixes.push(`cut ${links.removed.length} link(s) not on the allowlist`)
      this.logger?.warn({ removed: links.removed }, 'safety: cut links from an outbound draft')
    }
    const cleaned = links.text

    const result = await this.outboundDecision(cleaned, links.removed.length > 0)
    if (context) {
      if (!result.ok) {
        await this.hold({
          direction: 'outbound',
          kind: context.kind ?? 'draft',
          context,
          category: result.category,
          reason: result.reason,
          by: result.by,
          text,
          ...(links.removed.length > 0 ? { links: links.removed } : {}),
        })
      } else if (links.removed.length > 0) {
        await this.hold({
          direction: 'outbound',
          kind: 'link',
          context,
          category: 'links',
          reason: links.removed.map((entry) => entry.reason).join('; '),
          by: 'links',
          text,
          links: links.removed,
        })
      }
    }
    return { decision: result, text: cleaned, fixes }
  }

  private async outboundDecision(cleaned: string, linksCut: boolean): Promise<SafetyDecision> {
    const match = screenText(cleaned, 'outbound')
    if (match) {
      this.logger?.warn({ category: match.category, term: match.term }, 'safety: outbound held by the screen')
      return { ok: false, category: match.category, reason: `contains "${match.term}"`, by: 'screen' }
    }
    // Nothing left to say once the links are gone is not a message to send.
    if (linksCut && !/[a-z0-9]/i.test(cleaned)) {
      return { ok: false, category: 'links', reason: 'the draft was only a disallowed link', by: 'links' }
    }
    return this.ask(cleaned, 'outbound')
  }

  private async ask(text: string, direction: Direction): Promise<SafetyDecision> {
    if (!this.judge || !text.trim()) return { ok: true }
    const verdict = await this.judge.judge({ text, direction })
    if (verdict.verdict === 'clean') return { ok: true }
    const category = verdict.category ?? 'unknown'
    this.logger?.[direction === 'outbound' ? 'warn' : 'info'](
      { direction, category, reason: verdict.reason },
      `safety: ${direction} held by the judge`,
    )
    return { ok: false, category, reason: verdict.reason || 'the judge held it', by: 'judge' }
  }
}

/** A gate with no judge and the default allowlist: the lexicon and the links only. */
export function lexicalGate(options: Omit<SafetyGateOptions, 'judge'> = {}): SafetyGate {
  return new SafetyGate(options)
}
