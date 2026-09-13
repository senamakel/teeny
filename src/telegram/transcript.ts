import { MongoClient, type Collection } from 'mongodb'
import type { ChatTurn } from '../types.js'

/** One line of a chat as stored: a turn, plus where and when. */
export interface TranscriptEntry extends ChatTurn {
  chatId: string
  /** The channel's own message id, so a redelivered update is stored once. */
  messageId: string
  at: string
}

/**
 * The verbatim recent history of every chat the account is in.
 *
 * Long-term memory (CortexDB) is semantic and budgeted: it answers "what do I
 * know about this person and topic". It is the wrong tool for "what were the
 * last forty lines in this room", which is what makes a reply land in a
 * conversation rather than beside it. Telegram's Bot API has no way to read
 * history back, so the only transcript the account will ever have is the one
 * it keeps itself, from the moment it starts listening.
 */
export interface ChatTranscript {
  append(entry: TranscriptEntry): Promise<void>
  /**
   * The most recent turns of a chat that fit the token budget, oldest first.
   * `before` excludes the message being answered, which is in the prompt in
   * full already.
   */
  recent(chatId: string, options: { maxTokens: number; before?: string }): Promise<ChatTurn[]>
  close(): Promise<void>
}

/**
 * A rough token count, good enough for a budget. Four characters per token
 * is the usual English estimate; chat is shorter words and more punctuation,
 * so it errs a little high, which is the safe side of a budget.
 */
export function estimateTokens(text: string): number {
  return Math.ceil(text.length / 4)
}

/** Per-turn overhead for the "- who: " prefix and the line break. */
const TURN_OVERHEAD_TOKENS = 6

/**
 * Walks newest-first, keeping turns until the budget is spent, then returns
 * what was kept in reading order. A single turn larger than the whole budget
 * is still kept if it is the most recent one, so a long paste does not blank
 * the history entirely.
 */
export function fitToBudget(newestFirst: ChatTurn[], maxTokens: number): ChatTurn[] {
  const kept: ChatTurn[] = []
  let spent = 0
  for (const turn of newestFirst) {
    const cost = estimateTokens(turn.text) + TURN_OVERHEAD_TOKENS
    if (spent + cost > maxTokens && kept.length > 0) break
    kept.push(turn)
    spent += cost
  }
  return kept.reverse()
}

/**
 * How many rows to read per query. Well above what a 5k-token budget can
 * hold, so the budget is the limit and this is only a guard against a chat
 * with a very long tail of one-word messages.
 */
const MAX_ROWS = 400

export class MongoChatTranscript implements ChatTranscript {
  private readonly client: MongoClient
  private readonly entries: Collection<TranscriptEntry>
  private indexed = false

  constructor(options: { uri: string; database: string; collection?: string }) {
    this.client = new MongoClient(options.uri)
    this.entries = this.client
      .db(options.database)
      .collection<TranscriptEntry>(options.collection ?? 'telegram_transcript')
  }

  private async ready(): Promise<void> {
    if (this.indexed) return
    await this.client.connect()
    await this.entries.createIndex({ chatId: 1, at: -1 })
    await this.entries.createIndex({ chatId: 1, messageId: 1 }, { unique: true })
    this.indexed = true
  }

  async append(entry: TranscriptEntry): Promise<void> {
    await this.ready()
    // Telegram redelivers an update the bot did not acknowledge in time, so
    // a line already stored is left alone rather than doubled.
    await this.entries.updateOne(
      { chatId: entry.chatId, messageId: entry.messageId },
      { $setOnInsert: entry },
      { upsert: true },
    )
  }

  async recent(
    chatId: string,
    options: { maxTokens: number; before?: string },
  ): Promise<ChatTurn[]> {
    await this.ready()
    const rows = await this.entries
      .find({ chatId, ...(options.before ? { messageId: { $ne: options.before } } : {}) })
      .sort({ at: -1 })
      .limit(MAX_ROWS)
      .toArray()
    return fitToBudget(
      rows.map(({ from, authorName, text }) => ({
        from,
        ...(authorName ? { authorName } : {}),
        text,
      })),
      options.maxTokens,
    )
  }

  async close(): Promise<void> {
    await this.client.close()
  }
}

/** The in-process fallback: what the bot had before there was a store. */
export class MemoryChatTranscript implements ChatTranscript {
  private readonly chats = new Map<string, TranscriptEntry[]>()

  async append(entry: TranscriptEntry): Promise<void> {
    const entries = this.chats.get(entry.chatId) ?? []
    if (entries.some((existing) => existing.messageId === entry.messageId)) return
    entries.push(entry)
    this.chats.set(entry.chatId, entries.slice(-MAX_ROWS))
  }

  async recent(
    chatId: string,
    options: { maxTokens: number; before?: string },
  ): Promise<ChatTurn[]> {
    const entries = (this.chats.get(chatId) ?? [])
      .filter((entry) => entry.messageId !== options.before)
      .slice()
      .reverse()
    return fitToBudget(
      entries.map(({ from, authorName, text }) => ({
        from,
        ...(authorName ? { authorName } : {}),
        text,
      })),
      options.maxTokens,
    )
  }

  async close(): Promise<void> {}
}
