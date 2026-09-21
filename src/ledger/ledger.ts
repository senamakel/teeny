import { createHash } from 'node:crypto'
import { MongoClient, type Collection } from 'mongodb'
import { redactHandles } from '../journal/journal.js'

/**
 * The inference ledger: one row per model call, and a running count of every
 * token ever spent.
 *
 * The journal records turns -- what the account was asked, what it thought,
 * what it said. This records calls: the proof, underneath every turn, that a
 * model actually ran. A ReAct turn is several rows here, one per step, and
 * each row carries what the model was shown (short of the system prompt),
 * what it reasoned, what came back, and how many tokens that cost.
 *
 * The system prompt is never stored and never served. What is stored is a
 * digest of the whole request, prompt included, so a row commits to the exact
 * prompt without revealing it, and a second digest of the prompt alone, so a
 * reader can see that every call ran under the same one.
 */

/** Who spent the tokens: the mascot's own surfaces, or the brain's agents. */
export type Lane = 'mascot' | 'brain'

export const LANES: Lane[] = ['mascot', 'brain']

export interface Tokens {
  input: number
  output: number
  /** Output tokens the provider reported as reasoning. Part of `output`. */
  reasoning: number
  /** Input tokens served from the provider's cache. Part of `input`. */
  cached: number
  total: number
}

export function zeroTokens(): Tokens {
  return { input: 0, output: 0, reasoning: 0, cached: 0, total: 0 }
}

export function addTokens(a: Tokens, b: Tokens): Tokens {
  return {
    input: a.input + b.input,
    output: a.output + b.output,
    reasoning: a.reasoning + b.reasoning,
    cached: a.cached + b.cached,
    total: a.total + b.total,
  }
}

export interface LedgerMessage {
  role: 'user' | 'assistant' | 'tool'
  /** The tool a `tool` message answers, or the tools an `assistant` message called. */
  tool?: string
  text: string
}

export interface Inference {
  id: string
  at: string
  lane: Lane
  /**
   * What the call was for. On the mascot's lane a surface: `reply`, `chat`,
   * `chart`, `morning`, `moon`, `quote`, `calendar`, `task`, `judge`, `critic`,
   * `vision`, `media-gate`. On the brain's lane the agent's id.
   */
  source: string
  /** The brain's own word for the call: `inference`, `planningCall`, `judgeCall`... */
  kind?: string
  model: string
  /**
   * Everything the model was shown except the system prompt: the task, then
   * the tool trail so far. Handles are blanked and long messages are cut.
   * Absent on a brain row, which is an accounting sample, not a transcript.
   */
  input?: LedgerMessage[]
  /** The model's reasoning, when the provider returns it as text. */
  reasoning?: string
  /** What came back: text, or the tool calls it asked for. */
  output?: string
  tokens: Tokens
  /** The provider's id for the response, when it sent one. */
  responseId?: string
  /** sha256 of the exact request, system prompt included, and the exact response. */
  digest?: string
  /** sha256 of the system prompt alone. */
  promptDigest?: string
  durationMs?: number
}

export type InferenceInput = Omit<Inference, 'id' | 'at'> & { id?: string; at?: string }

export interface LaneTotals {
  calls: number
  tokens: Tokens
  /** The oldest and newest call counted, ISO. Absent until one is. */
  first?: string
  last?: string
}

export interface Totals {
  lanes: Record<Lane, LaneTotals>
  /** By lane and source, `mascot/reply`, `brain/scout`. */
  sources: Record<string, LaneTotals>
}

export function emptyTotals(): Totals {
  return {
    lanes: { mascot: { calls: 0, tokens: zeroTokens() }, brain: { calls: 0, tokens: zeroTokens() } },
    sources: {},
  }
}

export interface InferenceLedger {
  /** Best-effort by contract: a ledger that cannot be written must never take a reply down. */
  record(entry: InferenceInput): Promise<void>
  /** Newest first. */
  recent(limit: number, filter?: { lane?: Lane; before?: string }): Promise<Inference[]>
  totals(): Promise<Totals>
}

function complete(entry: InferenceInput): Inference {
  return {
    ...entry,
    id: entry.id ?? `i-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`,
    at: entry.at ?? new Date().toISOString(),
  }
}

function fold(into: LaneTotals, entry: Inference): void {
  into.calls += 1
  into.tokens = addTokens(into.tokens, entry.tokens)
  if (!into.first || entry.at < into.first) into.first = entry.at
  if (!into.last || entry.at > into.last) into.last = entry.at
}

export function sourceKey(entry: Pick<Inference, 'lane' | 'source'>): string {
  return `${entry.lane}/${entry.source}`
}

/** For tests and for running without Mongo. Keeps the last few thousand rows; the totals keep everything. */
export class MemoryLedger implements InferenceLedger {
  private readonly entries: Inference[] = []
  private readonly sums = emptyTotals()

  constructor(private readonly capacity = 5_000) {}

  async record(entry: InferenceInput): Promise<void> {
    const inference = complete(entry)
    if (this.entries.some((existing) => existing.id === inference.id)) return
    this.entries.unshift(inference)
    this.entries.length = Math.min(this.entries.length, this.capacity)
    fold(this.sums.lanes[inference.lane], inference)
    const key = sourceKey(inference)
    this.sums.sources[key] ??= { calls: 0, tokens: zeroTokens() }
    fold(this.sums.sources[key], inference)
  }

  async recent(limit: number, filter: { lane?: Lane; before?: string } = {}): Promise<Inference[]> {
    return [...this.entries]
      .filter((entry) => (filter.lane ? entry.lane === filter.lane : true))
      .filter((entry) => (filter.before ? entry.at < filter.before : true))
      .sort((a, b) => (a.at < b.at ? 1 : a.at > b.at ? -1 : 0))
      .slice(0, limit)
  }

  async totals(): Promise<Totals> {
    return structuredClone(this.sums)
  }
}

interface TotalsDoc {
  _id: string
  lanes: Record<Lane, LaneTotals>
  sources: Record<string, LaneTotals>
}

/**
 * The ledger in the same Mongo the journal lives in.
 *
 * Rows go in one collection; the totals are one document updated with `$inc`
 * on every row, so the counter on the page is one read whatever the row
 * count is, and it keeps counting after old rows are gone. A source key
 * carries a slash, which Mongo allows in a field name; a dot would not be.
 */
export class MongoLedger implements InferenceLedger {
  private readonly client: MongoClient
  private readonly rows: Collection<Inference>
  private readonly sums: Collection<TotalsDoc>
  private indexed = false

  constructor(options: { uri: string; database: string; collection?: string }) {
    this.client = new MongoClient(options.uri)
    const db = this.client.db(options.database)
    const name = options.collection ?? 'inferences'
    this.rows = db.collection<Inference>(name)
    this.sums = db.collection<TotalsDoc>(`${name}_totals`)
  }

  private async ready(): Promise<void> {
    if (this.indexed) return
    await this.client.connect()
    await this.rows.createIndex({ id: 1 }, { unique: true })
    await this.rows.createIndex({ at: -1 })
    await this.rows.createIndex({ lane: 1, at: -1 })
    this.indexed = true
  }

  async record(entry: InferenceInput): Promise<void> {
    await this.ready()
    const inference = complete(entry)
    // Insert-if-absent, and count only what was inserted: a retried fold of
    // the brain's samples must not count a sample twice.
    const result = await this.rows.updateOne(
      { id: inference.id },
      { $setOnInsert: inference },
      { upsert: true },
    )
    if (!result.upsertedCount) return
    const { tokens } = inference
    const inc: Record<string, number> = {}
    const min: Record<string, string> = {}
    const max: Record<string, string> = {}
    for (const prefix of [`lanes.${inference.lane}`, `sources.${sourceKey(inference)}`]) {
      inc[`${prefix}.calls`] = 1
      for (const field of ['input', 'output', 'reasoning', 'cached', 'total'] as const) {
        inc[`${prefix}.tokens.${field}`] = tokens[field]
      }
      min[`${prefix}.first`] = inference.at
      max[`${prefix}.last`] = inference.at
    }
    await this.sums.updateOne({ _id: 'totals' }, { $inc: inc, $min: min, $max: max }, { upsert: true })
  }

  async recent(limit: number, filter: { lane?: Lane; before?: string } = {}): Promise<Inference[]> {
    await this.ready()
    return this.rows
      .find(
        {
          ...(filter.lane ? { lane: filter.lane } : {}),
          ...(filter.before ? { at: { $lt: filter.before } } : {}),
        },
        { projection: { _id: 0 } },
      )
      .sort({ at: -1 })
      .limit(limit)
      .toArray()
  }

  async totals(): Promise<Totals> {
    await this.ready()
    const doc = await this.sums.findOne({ _id: 'totals' })
    const totals = emptyTotals()
    if (!doc) return totals
    for (const lane of LANES) {
      const stored = doc.lanes?.[lane]
      if (stored) totals.lanes[lane] = { ...stored, tokens: { ...zeroTokens(), ...stored.tokens } }
    }
    for (const [key, stored] of Object.entries(doc.sources ?? {})) {
      totals.sources[key] = { ...stored, tokens: { ...zeroTokens(), ...stored.tokens } }
    }
    return totals
  }

  async close(): Promise<void> {
    await this.client.close()
  }
}

// --- shaping a row -----------------------------------------------------------

/** Longer than this and a message on the page is a wall, not a record. */
export const MAX_MESSAGE_CHARS = 2_000
/** A run's tool trail can be long; the page shows the tail, which is what the model answered to. */
export const MAX_MESSAGES = 12

export function cut(text: string, max = MAX_MESSAGE_CHARS): string {
  const flat = text.replace(/\r/g, '').trim()
  return flat.length > max ? `${flat.slice(0, max - 1)}…` : flat
}

export function sha256(...parts: string[]): string {
  const hash = createHash('sha256')
  for (const part of parts) hash.update(part)
  return hash.digest('hex')
}

/**
 * A raw OpenAI-shaped `usage` block, as every chat completion returns one,
 * read into the ledger's own count. The details blocks are optional on the
 * wire and absent from most compatible endpoints, and count as zero then.
 */
export function tokensFromUsage(usage: unknown): Tokens {
  const u = (usage ?? {}) as Record<string, unknown>
  const n = (value: unknown): number => (typeof value === 'number' && Number.isFinite(value) ? value : 0)
  const details = (value: unknown): Record<string, unknown> =>
    value && typeof value === 'object' ? (value as Record<string, unknown>) : {}
  const input = n(u.prompt_tokens ?? u.input_tokens)
  const output = n(u.completion_tokens ?? u.output_tokens)
  const reasoning = n(
    details(u.completion_tokens_details).reasoning_tokens ??
      details(u.output_tokens_details).reasoning_tokens ??
      details(u.output_token_details).reasoning,
  )
  const cached = n(
    details(u.prompt_tokens_details).cached_tokens ??
      details(u.input_tokens_details).cached_tokens ??
      details(u.input_token_details).cache_read,
  )
  return { input, output, reasoning, cached, total: n(u.total_tokens) || input + output }
}

/** The public shape of a row: handles blanked, nothing else changed. */
export function redactInference(entry: Inference): Inference {
  return {
    ...entry,
    ...(entry.input ? { input: entry.input.map((m) => ({ ...m, text: redactHandles(m.text) })) } : {}),
    ...(entry.reasoning !== undefined ? { reasoning: redactHandles(entry.reasoning) } : {}),
    ...(entry.output !== undefined ? { output: redactHandles(entry.output) } : {}),
  }
}

/**
 * Records a row and swallows the failure, logging it. Every caller wants the
 * same thing: a ledger that is down costs a line in the log, never a reply.
 */
export async function recordQuietly(
  ledger: InferenceLedger | undefined,
  entry: InferenceInput,
  log?: (message: string, error: unknown) => void,
): Promise<void> {
  if (!ledger) return
  try {
    await ledger.record(entry)
  } catch (error) {
    ;(log ?? ((message, err) => console.warn(message, err)))('inference ledger write failed', error)
  }
}
