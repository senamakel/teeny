import { MongoClient, type Collection } from 'mongodb'

export type Trigger = 'mention' | 'chart' | 'task'
export type Tone = 'friendly' | 'neutral' | 'negative' | 'hostile'
export type Outcome = 'posted' | 'dry-run' | 'skipped' | 'held' | 'failed'

export interface ToolCall {
  name: string
  /** One line on what came back, never the raw payload. */
  result: string
}

/**
 * One turn of the account's mind, as published on the landing page.
 *
 * This is the record of what the account *did*, kept apart from what it
 * *remembers*: memory is what it carries into the next conversation and is
 * deliberately selective (sour posts are answered, never stored), while the
 * journal is the audit trail and keeps everything, including the turns where
 * it decided to say nothing.
 */
export interface Reflection {
  id: string
  at: string
  trigger: Trigger
  /** What arrived. Handles are redacted before this leaves the box. */
  incoming: string
  tone: Tone
  /** What memory said, summarised to a few lines. */
  recalled: string[]
  tools: ToolCall[]
  /** The reasoning, in the account's own words. */
  thinking: string
  /** The reply, chart caption, or task output. Empty when it stayed quiet. */
  said: string
  /** Things it decided against, and why. */
  declined: string[]
  outcome: Outcome
  outcomeNote?: string
}

export type ReflectionInput = Omit<Reflection, 'id' | 'at'> & { id?: string; at?: string }

export interface Journal {
  /** Best-effort by contract: a journal that cannot be written must never take a reply down. */
  record(entry: ReflectionInput): Promise<void>
  /** Newest first. */
  recent(limit: number): Promise<Reflection[]>
}

function complete(entry: ReflectionInput): Reflection {
  return {
    ...entry,
    id: entry.id ?? `r-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`,
    at: entry.at ?? new Date().toISOString(),
  }
}

/** The mind, in the same Mongo the mention queue lives in. */
export class MongoJournal implements Journal {
  private readonly client: MongoClient
  private readonly reflections: Collection<Reflection>
  private indexed = false

  constructor(options: { uri: string; database: string; collection?: string }) {
    this.client = new MongoClient(options.uri)
    this.reflections = this.client
      .db(options.database)
      .collection<Reflection>(options.collection ?? 'reflections')
  }

  private async ready(): Promise<void> {
    if (this.indexed) return
    await this.client.connect()
    await this.reflections.createIndex({ id: 1 }, { unique: true })
    await this.reflections.createIndex({ at: -1 })
    this.indexed = true
  }

  async record(entry: ReflectionInput): Promise<void> {
    await this.ready()
    const reflection = complete(entry)
    // Insert-if-absent, so a retried turn cannot journal itself twice.
    await this.reflections.updateOne(
      { id: reflection.id },
      { $setOnInsert: reflection },
      { upsert: true },
    )
  }

  async recent(limit: number): Promise<Reflection[]> {
    await this.ready()
    return this.reflections
      .find({}, { projection: { _id: 0 } })
      .sort({ at: -1 })
      .limit(limit)
      .toArray()
  }

  async close(): Promise<void> {
    await this.client.close()
  }
}

/** For tests and for running without Mongo. Keeps the last few hundred. */
export class MemoryJournal implements Journal {
  private readonly entries: Reflection[] = []

  constructor(private readonly capacity = 500) {}

  async record(entry: ReflectionInput): Promise<void> {
    const reflection = complete(entry)
    if (this.entries.some((existing) => existing.id === reflection.id)) return
    this.entries.unshift(reflection)
    this.entries.length = Math.min(this.entries.length, this.capacity)
  }

  async recent(limit: number): Promise<Reflection[]> {
    return [...this.entries]
      .sort((a, b) => (a.at < b.at ? 1 : a.at > b.at ? -1 : 0))
      .slice(0, limit)
  }
}

/**
 * The page shows what the account thought, not who it was talking to. A
 * handle in the incoming text or the reply is somebody else's name on a page
 * they never asked to be on, so every one is blanked before it leaves.
 */
export function redactHandles(text: string): string {
  return text.replace(/@[A-Za-z0-9_]{1,15}\b/g, '@…')
}

export function redactReflection(reflection: Reflection): Reflection {
  return {
    ...reflection,
    incoming: redactHandles(reflection.incoming),
    said: redactHandles(reflection.said),
    thinking: redactHandles(reflection.thinking),
    recalled: reflection.recalled.map(redactHandles),
    declined: reflection.declined.map(redactHandles),
    tools: reflection.tools.map((call) => ({ ...call, result: redactHandles(call.result) })),
  }
}

const MAX_LINE = 180

/** A memory context block, cut down to the few lines a reader can take in. */
export function summariseRecall(block: string, maxLines = 4): string[] {
  return block
    .split('\n')
    .map((line) => line.replace(/^[\s\-*•>#]+/, '').replace(/\s+/g, ' ').trim())
    // Section headers like "ABOUT @fren:" or "GENERAL CONTEXT:" are structure, not memory.
    .filter((line) => line.length > 0 && !/^[A-Z][A-Z\s]*(?:@\S+)?:$/.test(line))
    .slice(0, maxLines)
    .map((line) => (line.length > MAX_LINE ? `${line.slice(0, MAX_LINE - 1)}…` : line))
}

/** A tool's output, as one line. */
export function summariseResult(output: unknown, max = 160): string {
  const text =
    typeof output === 'string'
      ? output
      : output === undefined
        ? ''
        : (() => {
            try {
              return JSON.stringify(output)
            } catch {
              return String(output)
            }
          })()
  const flat = text.replace(/\s+/g, ' ').trim()
  return flat.length > max ? `${flat.slice(0, max - 1)}…` : flat
}
