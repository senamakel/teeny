import { V1Client } from 'cortexdbai/v1'
import { personOf, type Memory, type Person, type XPost } from '../types.js'

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

export class CortexMemory implements Memory {
  private readonly client: V1Client

  constructor(
    options: {
      apiUrl: string
      actor: string
      bearer: string
    },
    private readonly scope: string,
  ) {
    this.client = new V1Client(options)
  }

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
    const { person } = options
    // Two passes, because they answer different questions: what the account
    // knows about this person, and what it knows about the topic. One query
    // over everything buries a quiet person's history under general context.
    const [about, general] = await Promise.all([
      person
        ? this.recallPack(`Everything known about this person and past exchanges with them.`, {
            metadata: { observed_actor: person.id },
          })
        : Promise.resolve(''),
      this.recallPack(query),
    ])

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
    const pack = await this.client.recall(this.scope, {
      query,
      view: 'holistic',
      include: ['events', 'facts', 'beliefs', 'episodes'],
      diagnostics: 'none',
      budgets: { max_tokens: metadata ? 600 : 1_200 },
      ...(metadata ? { filters: metadata } : {}),
    })
    return pack.context_block
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
}
