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
      by: 'screen' | 'judge'
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
  logger?: SafetyLogger
}

/** The two lines the gate writes: a hold, and an edit. Pino-shaped. */
export interface SafetyLogger {
  info(detail: Record<string, unknown>, message?: string): void
  warn(detail: Record<string, unknown>, message?: string): void
}

export class SafetyGate {
  private readonly judge: Judge | undefined
  private readonly allowlist: AllowlistEntry[]
  private readonly logger: SafetyLogger | undefined

  constructor(options: SafetyGateOptions = {}) {
    this.judge = options.judge
    this.allowlist = parseAllowlist([...DEFAULT_LINK_ALLOWLIST, ...(options.extraAllowedLinks ?? [])])
    this.logger = options.logger
  }

  /** The lexicon alone, synchronous. For scrubbing context lines cheaply. */
  screen(text: string, direction: Direction = 'inbound'): boolean {
    return screenText(text, direction) === undefined
  }

  /** Whether a message from somebody else is something the account processes at all. */
  async inbound(text: string): Promise<SafetyDecision> {
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
   */
  async outbound(text: string): Promise<OutboundResult> {
    const fixes: string[] = []
    const links = filterLinks(text, this.allowlist)
    if (links.removed.length > 0) {
      fixes.push(`cut ${links.removed.length} link(s) not on the allowlist`)
      this.logger?.warn({ removed: links.removed }, 'safety: cut links from an outbound draft')
    }
    const cleaned = links.text

    const match = screenText(cleaned, 'outbound')
    if (match) {
      this.logger?.warn({ category: match.category, term: match.term }, 'safety: outbound held by the screen')
      return {
        decision: { ok: false, category: match.category, reason: `contains "${match.term}"`, by: 'screen' },
        text: cleaned,
        fixes,
      }
    }
    // Nothing left to say once the links are gone is not a message to send.
    if (links.removed.length > 0 && !/[a-z0-9]/i.test(cleaned)) {
      return {
        decision: { ok: false, category: 'links', reason: 'the draft was only a disallowed link', by: 'screen' },
        text: cleaned,
        fixes,
      }
    }
    const decision = await this.ask(cleaned, 'outbound')
    return { decision, text: cleaned, fixes }
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
