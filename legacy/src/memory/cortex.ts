import { V1Client } from 'cortexdbai/v1'
import { cutAt } from './text.js'
import {
  chatPersonOf,
  personOf,
  type ChatMemory,
  type ChatMessage,
  type Memory,
  type Person,
  type XPost,
} from '../types.js'

/**
 * The shape of a captured record is part of its identity. Bumping this makes
 * the new representation a new record rather than a 409 against the old one,
 * which is what CortexDB returns when a key is reused with a different body.
 */
const CAPTURE_VERSION = 2

/** A conflict means the record is already stored, which is not a failure. */
function ignoreIdempotencyConflict(error: unknown): void {
  const message = error instanceof Error ? error.message : String(error)
  if (!message.includes('IDEMPOTENCY_CONFLICT')) throw error
}

/**
 * Recall reads on the turn's critical path; captures write after it. They get
 * separate clients so each can carry its own deadline: the SDK's default is a
 * minute for everything, and a reply that waits a minute on a recall which
 * then times out is a reply drafted a minute late with no memory anyway.
 */
export interface CortexTimeouts {
  recallMs?: number
  captureMs?: number
  /**
   * Whether to run the second pack, about the person being answered. It
   * filters on the person's `@handle` label, which every mention and reply
   * carries: measured 2026-10-05 at under two seconds, returning that
   * person's actual exchanges. The `observed_actor` filter it replaced never
   * came back on the shared v0.9.9 store.
   */
  personPass?: boolean
  /**
   * How long recall stays off after one times out. A recall the client gives
   * up on keeps running on the store, and on 2026-10-05 a handful of them --
   * one per reply -- stalled every read until a restart. Backing off is what
   * stops the account from being the thing that keeps the store down.
   */
  cooldownMs?: number
  /** Recalls allowed in flight at once; past it a recall fails fast. */
  maxInFlight?: number
}

function isTimeout(error: unknown): boolean {
  return error instanceof Error && /timed out/i.test(error.message)
}

/** Recall is off for now, after a timeout or because too many are in flight. */
export class RecallPaused extends Error {}

/**
 * What a recall may draw on: conversation, notes the account left itself, and
 * the account's own posts. Everything else in the scope is webhook activity --
 * raw like, repost and follow payloads, two thirds of it on 2026-10-05 -- and a
 * pack that fills with JSON about who liked what has no room left for what
 * anybody said.
 */
export const RECALL_LABELS = [
  'mention',
  'reply',
  'message',
  'agent_note',
  'chart_post',
  'morning_post',
  'moon_post',
  'quote_post',
  'lore_post',
  'article',
  'narrative_episode',
  'calendar_post',
]

/**
 * Generous for prose, so the cap only bites when the budget was ignored: a
 * pack is mostly English and JSON, which run three to four characters a token.
 */
const CHARS_PER_TOKEN = 6
const PACK_SEPARATOR = '\n\n---\n\n'

/**
 * The store's token budget is a request, not a guarantee. A scope whose
 * derived layers were still being built answered a 1,200-token holistic recall
 * with its entire event log -- 3.9 million characters -- which as a prompt is
 * not a worse reply but no reply at all. Cut at an item boundary, newest-first
 * order kept, so what survives is whole memories rather than half of one.
 */
export function capPack(block: string, maxChars: number): string {
  if (block.length <= maxChars) return block
  const kept: string[] = []
  let length = 0
  for (const item of block.split(PACK_SEPARATOR)) {
    const added = item.length + (kept.length ? PACK_SEPARATOR.length : 0)
    if (length + added > maxChars) break
    kept.push(item)
    length += added
  }
  return kept.length ? kept.join(PACK_SEPARATOR) : cutAt(block, maxChars)
}

export class CortexMemory implements Memory, ChatMemory {
  private readonly client: V1Client
  private readonly reader: V1Client

  constructor(
    options: {
      apiUrl: string
      actor: string
      bearer: string
      /** For tests; the SDK uses the global fetch otherwise. */
      fetch?: typeof fetch
    },
    private readonly scope: string,
    timeouts: CortexTimeouts = {},
  ) {
    this.client = new V1Client({ ...options, ...(timeouts.captureMs ? { timeoutMs: timeouts.captureMs } : {}) })
    this.reader = new V1Client({ ...options, ...(timeouts.recallMs ? { timeoutMs: timeouts.recallMs } : {}) })
    this.personPass = timeouts.personPass ?? true
    this.cooldownMs = timeouts.cooldownMs ?? 5 * 60_000
    this.maxInFlight = timeouts.maxInFlight ?? 2
  }

  private readonly personPass: boolean
  private readonly cooldownMs: number
  private readonly maxInFlight: number
  private pausedUntil = 0
  private inFlight = 0
  private readonly now: () => number = Date.now

  async captureActivity(
    kind: string,
    id: string,
    payload: unknown,
    person?: Person,
  ): Promise<void> {
    await this.client.experience(
      this.scope,
      {
        jsonData: { kind, payload },
        labels: ['x', 'activity', kind, ...(person?.username ? [`@${person.username}`] : [])],
        observedAt: new Date().toISOString(),
        idempotencyKey: `x:${kind}:${CAPTURE_VERSION}:${id}`,
        ...(person ? { observedActor: { id: person.id, type: 'person' } } : {}),
      },
      { wait: 'captured' },
    ).catch(ignoreIdempotencyConflict)
  }

  /**
   * A mention is a message from a person, so it is stored as one: a chat turn
   * whose `observedActor` is the author. Stored as an opaque JSON blob under
   * the agent's own id instead, every conversation belongs to the bot and
   * nobody can be told apart on recall.
   */
  async captureMention(post: XPost): Promise<void> {
    const person = personOf(post)
    await this.client.experience(
      this.scope,
      {
        text: post.text,
        role: 'user',
        labels: [
          'x',
          'mention',
          ...(person?.username ? [`@${person.username}`] : []),
          ...(post.conversationId ? [`thread:${post.conversationId}`] : []),
        ],
        observedAt: post.createdAt ?? new Date().toISOString(),
        idempotencyKey: `x:mention:${CAPTURE_VERSION}:${post.id}`,
        ...(person ? { observedActor: { id: person.id, type: 'person' } } : {}),
      },
      { wait: 'captured' },
    ).catch(ignoreIdempotencyConflict)
  }

  async recall(query: string, options: { person?: Person } = {}): Promise<string> {
    if (this.now() < this.pausedUntil) {
      throw new RecallPaused(`recall paused for ${Math.ceil((this.pausedUntil - this.now()) / 1_000)}s after a timeout`)
    }
    if (this.inFlight >= this.maxInFlight) throw new RecallPaused('too many recalls in flight')
    this.inFlight += 1
    try {
      return await this.recallOnce(query, options)
    } catch (error) {
      if (isTimeout(error)) this.pausedUntil = this.now() + this.cooldownMs
      throw error
    } finally {
      this.inFlight -= 1
    }
  }

  private async recallOnce(query: string, options: { person?: Person }): Promise<string> {
    // Only the handle can find a person: it is the label every exchange with
    // them carries. A person known by id alone is recalled about generally.
    const person = options.person?.username ? options.person : undefined
    if (person && !this.personPass) {
      // The handle leads the query bare. Phrased as a subject -- "About @ana:",
      // "Regarding @ana", "Everything known about this person" -- the store
      // treats it as a request for that entity's dossier, and on v0.9.9 that
      // path does not come back (measured 2026-10-05: 15s+ every time, against
      // under a second for the same words without the "About").
      const general = await this.recallPack(`@${person.username} ${query}`)
      return general.trim() ? `GENERAL CONTEXT:\n${general}` : ''
    }
    // Two passes, because they answer different questions: what the account
    // knows about this person, and what it knows about the topic. One query
    // over everything buries a quiet person's history under general context.
    //
    // Settled rather than all-or-nothing: the person pass is the slower of the
    // two (a filtered holistic pack), and losing it should cost the turn its
    // history with this person, not its general context too. Only when every
    // pass that ran fails does the recall fail, so the caller can say memory
    // was unavailable instead of reading the silence as "nothing is known".
    const [aboutResult, generalResult] = await Promise.allSettled([
      person
        ? this.recallPack(`What has this person said, and what did you say back?`, {
            metadata: { labels: `@${person.username}` },
          })
        : Promise.resolve(''),
      this.recallPack(query),
    ])
    // A pass that timed out left a query running on the store even when the
    // other pass answered; the breaker counts it, or a half-answered recall
    // per reply keeps adding to the pile it exists to stop.
    for (const result of [aboutResult, generalResult]) {
      if (result.status === 'rejected' && isTimeout(result.reason)) this.pausedUntil = this.now() + this.cooldownMs
    }
    if (generalResult.status === 'rejected' && (!person || aboutResult.status === 'rejected')) {
      throw generalResult.reason
    }
    const about = aboutResult.status === 'fulfilled' ? aboutResult.value : ''
    const general = generalResult.status === 'fulfilled' ? generalResult.value : ''

    const sections = []
    if (about.trim()) {
      sections.push(
        `ABOUT ${person?.username ? `@${person.username}` : 'this person'}:\n${about}`,
      )
    }
    if (general.trim()) sections.push(`GENERAL CONTEXT:\n${general}`)
    return sections.join('\n\n')
  }

  private async recallPack(
    query: string,
    metadata?: { metadata: Record<string, string | string[]> },
  ): Promise<string> {
    const pack = await this.reader.recall(this.scope, {
      query,
      view: 'holistic',
      include: ['events', 'facts', 'beliefs', 'episodes'],
      diagnostics: 'none',
      budgets: { max_tokens: metadata ? 600 : 1_200 },
      filters: metadata ?? { metadata: { labels: RECALL_LABELS } },
    })
    return capPack(pack.context_block ?? '', (metadata ? 600 : 1_200) * CHARS_PER_TOKEN)
  }

  /** The bot's own turn in the same conversation, kept about the same person. */
  async captureReply(input: XPost, reply: string, postedId?: string): Promise<void> {
    const person = personOf(input)
    await this.client.experience(
      this.scope,
      {
        text: reply,
        role: 'assistant',
        labels: [
          'x',
          'reply',
          postedId ? 'posted' : 'dry-run',
          ...(person?.username ? [`@${person.username}`] : []),
          ...(input.conversationId ? [`thread:${input.conversationId}`] : []),
        ],
        observedAt: new Date().toISOString(),
        idempotencyKey: `x:reply:${CAPTURE_VERSION}:${input.id}:${postedId ?? 'dry-run'}`,
        ...(person ? { observedActor: { id: person.id, type: 'person' } } : {}),
      },
      { wait: 'captured' },
    ).catch(ignoreIdempotencyConflict)
  }

  /**
   * A Telegram message, stored the way a mention is: a chat turn from the
   * person who wrote it, in the same scope as everything from X. One store on
   * purpose -- somebody who asked about the roadmap on X last week and turns
   * up in a Telegram group today is the same person, and the account should
   * know it. The channel is a label, not a scope.
   */
  async captureChatMessage(message: ChatMessage): Promise<void> {
    const person = chatPersonOf(message)
    await this.client.experience(
      this.scope,
      {
        text: message.text,
        role: 'user',
        labels: [
          message.channel,
          'message',
          message.chatKind,
          ...(person.username ? [`@${person.username}`] : []),
          `chat:${message.channel}-${message.chatId}`,
        ],
        observedAt: message.createdAt ?? new Date().toISOString(),
        idempotencyKey: `${message.channel}:message:${CAPTURE_VERSION}:${message.chatId}:${message.id}`,
        observedActor: { id: person.id, type: 'person' },
      },
      { wait: 'captured' },
    ).catch(ignoreIdempotencyConflict)
  }

  async captureChatReply(message: ChatMessage, reply: string, sentId?: string): Promise<void> {
    const person = chatPersonOf(message)
    await this.client.experience(
      this.scope,
      {
        text: reply,
        role: 'assistant',
        labels: [
          message.channel,
          'reply',
          sentId ? 'sent' : 'dry-run',
          ...(person.username ? [`@${person.username}`] : []),
          `chat:${message.channel}-${message.chatId}`,
        ],
        observedAt: new Date().toISOString(),
        idempotencyKey: `${message.channel}:reply:${CAPTURE_VERSION}:${message.chatId}:${message.id}:${sentId ?? 'dry-run'}`,
        observedActor: { id: person.id, type: 'person' },
      },
      { wait: 'captured' },
    ).catch(ignoreIdempotencyConflict)
  }
}
