import type { FastifyBaseLogger } from 'fastify'
import { applyHouseStyle } from '../agent/house-style.js'
import {
  TelegramApiError,
  splitForTelegram,
  type TelegramApi,
  type TelegramMessage,
  type TelegramUpdate,
} from '../clients/telegram.js'
import type { ChatHarness } from '../harness/chat-harness.js'
import type { ChatMessage, GifSource } from '../types.js'
import { commandOf, isAddressedToBot, toChatMessage, type BotIdentity } from './inbox.js'
import { MemoryChatTranscript, type ChatTranscript } from './transcript.js'

export interface TelegramBotOptions {
  api: TelegramApi
  harness: ChatHarness
  logger: FastifyBaseLogger
  bot: BotIdentity
  /** Log the draft instead of sending it. */
  dryRun?: boolean
  gifs?: GifSource
  maxEmoji?: number
  /**
   * Messages older than this on arrival are acknowledged but not answered. A
   * restart after an hour down would otherwise greet everybody who spoke in
   * the meantime, an hour late, all at once.
   */
  maxMessageAgeMs?: number
  /** How long one getUpdates call may wait for something to happen. */
  pollTimeoutSeconds?: number
  /**
   * Where every line the account sees is kept, so a reply can read the
   * conversation it is joining. Defaults to in-process memory, which is lost
   * on restart; production passes the Mongo-backed one.
   */
  transcript?: ChatTranscript
  /** How much of the chat's history the drafter is shown, in tokens. */
  historyTokens?: number
  /**
   * How long to wait after a message for the same person to send another
   * before answering. People type in pieces -- "hey", "quick q", then the
   * question -- and answering each piece is three replies to one thought.
   * Zero answers immediately.
   */
  debounceMs?: number
  /** The least time between two GIFs in one chat. */
  gifCooldownMs?: number
  /** Injectable for the tests. */
  sleep?: (ms: number) => Promise<void>
  now?: () => number
}

const defaultSleep = (ms: number): Promise<void> =>
  new Promise((resolve) => setTimeout(resolve, ms))

/** One person's messages in one chat, waiting to be answered together. */
interface Batch {
  key: string
  raw: TelegramMessage[]
  messages: ChatMessage[]
  timer?: ReturnType<typeof setTimeout>
}

/**
 * The Telegram side of the account: one long-poll loop, one drafter per turn.
 *
 * Chats are handled one turn at a time each, and different chats in
 * parallel. A DM waiting on a slow group is a person left on read; two
 * replies to the same person racing each other is a conversation that reads
 * out of order. Both are avoided by serialising per chat and nowhere else.
 */
export class TelegramBot {
  private offset: number | undefined
  private stopped = false
  private readonly chains = new Map<string, Promise<void>>()
  private readonly batches = new Map<string, Batch>()
  private readonly lastGifAt = new Map<string, number>()
  private readonly transcript: ChatTranscript
  private readonly sleep: (ms: number) => Promise<void>
  private readonly now: () => number

  constructor(private readonly options: TelegramBotOptions) {
    this.transcript = options.transcript ?? new MemoryChatTranscript()
    this.sleep = options.sleep ?? defaultSleep
    this.now = options.now ?? Date.now
  }

  /** Runs until `stop()`; never throws. Every failure is logged and polled past. */
  async start(): Promise<void> {
    const { logger } = this.options
    while (!this.stopped) {
      try {
        const updates = await this.options.api.getUpdates({
          ...(this.offset === undefined ? {} : { offset: this.offset }),
          timeoutSeconds: this.options.pollTimeoutSeconds ?? 30,
        })
        for (const update of updates) {
          this.offset = update.update_id + 1
          void this.receive(update).catch((error: unknown) => {
            logger.error({ err: error, updateId: update.update_id }, 'telegram update failed')
          })
        }
      } catch (error) {
        // 409 is another process polling the same token. Backing off does not
        // fix it, but it keeps the log readable while somebody does.
        const retryAfter =
          error instanceof TelegramApiError && error.retryAfterSeconds
            ? error.retryAfterSeconds * 1_000
            : error instanceof TelegramApiError && error.code === 409
              ? 30_000
              : 5_000
        logger.error({ err: error, retryAfter }, 'telegram poll failed')
        await this.sleep(retryAfter)
      }
    }
  }

  stop(): void {
    this.stopped = true
  }

  /** Whatever is in flight, so a shutdown can wait for replies to leave. */
  async drain(): Promise<void> {
    for (const batch of this.batches.values()) this.flush(batch)
    await Promise.all(this.chains.values())
  }

  /**
   * One update in: kept in the transcript, and if it is for the account,
   * held for a moment in case more of the same thought is on its way.
   */
  async receive(update: TelegramUpdate): Promise<void> {
    const raw = update.message
    if (!raw) return
    const { logger, bot } = this.options
    const message = toChatMessage(raw, bot)
    if (!message) return

    // Seen is not the same as answered: a line in a group that was not for the
    // account still tells the next reply what the room was talking about, so
    // every line is kept whether or not this one gets an answer. Kept before
    // anything else so a failure further down does not lose it.
    await this.transcript.append({
      chatId: message.chatId,
      messageId: message.id,
      at: message.createdAt ?? new Date(this.now()).toISOString(),
      from: 'person',
      ...(message.authorName ? { authorName: message.authorName } : {}),
      text: message.text,
    })

    if (!isAddressedToBot(raw, bot)) return

    const maxAge = this.options.maxMessageAgeMs ?? 5 * 60_000
    const age = this.now() - raw.date * 1_000
    if (age > maxAge) {
      logger.info({ chatId: message.chatId, ageMs: age }, 'skipping a stale telegram message')
      return
    }

    // Batched per person per chat, not per chat: two people addressing the
    // account in the same few seconds are two conversations.
    const key = `${message.chatId}:${message.authorId}`
    const batch = this.batches.get(key) ?? { key, raw: [], messages: [] }
    batch.raw.push(raw)
    batch.messages.push(message)
    this.batches.set(key, batch)

    const debounce = this.options.debounceMs ?? 0
    if (batch.timer) clearTimeout(batch.timer)
    if (debounce <= 0) {
      this.flush(batch)
    } else {
      batch.timer = setTimeout(() => this.flush(batch), debounce)
    }
  }

  /** One update, received and answered, with no debounce. For the tests. */
  async handle(update: TelegramUpdate): Promise<void> {
    await this.receive(update)
    await this.drain()
  }

  /** Hands a batch to its chat's chain and forgets it. */
  private flush(batch: Batch): void {
    if (batch.timer) clearTimeout(batch.timer)
    if (this.batches.get(batch.key) !== batch) return
    this.batches.delete(batch.key)

    const chatKey = batch.messages[0]?.chatId ?? batch.key
    const previous = this.chains.get(chatKey) ?? Promise.resolve()
    const next = previous
      .then(() => this.answer(batch))
      .catch((error: unknown) => {
        this.options.logger.error(
          { err: error, chatId: chatKey, messageIds: batch.messages.map((m) => m.id) },
          'telegram message failed',
        )
      })
    this.chains.set(chatKey, next)
    void next.finally(() => {
      if (this.chains.get(chatKey) === next) this.chains.delete(chatKey)
    })
  }

  /** The account's own line in the transcript, so it can see what it said. */
  private async keepOwnTurn(chatId: string, messageId: string, text: string): Promise<void> {
    await this.transcript
      .append({ chatId, messageId, at: new Date(this.now()).toISOString(), from: 'bot', text })
      .catch((error: unknown) =>
        this.options.logger.warn({ err: error, chatId }, 'could not keep own turn in the transcript'),
      )
  }

  private gifAllowed(chatId: string): boolean {
    const cooldown = this.options.gifCooldownMs ?? 10 * 60_000
    const last = this.lastGifAt.get(chatId)
    return last === undefined || this.now() - last >= cooldown
  }

  /** One batch of messages from one person, answered as one turn. */
  private async answer(batch: Batch): Promise<void> {
    const { logger, api } = this.options
    const last = batch.raw[batch.raw.length - 1]
    const lastMessage = batch.messages[batch.messages.length - 1]
    if (!last || !lastMessage) return

    // The messages joined, one per line, presented as the last of them: the
    // reply threads under the latest and the drafter reads the whole thought.
    const message: ChatMessage = {
      ...lastMessage,
      text: batch.messages.map((m) => m.text).join('\n'),
    }
    const ids = batch.messages.map((m) => m.id)

    // A command other than /start is not something the account does. It is
    // answered as a message anyway, so a typo does not go unanswered.
    const command = commandOf(last)
    if (command && command !== 'start') {
      logger.info({ chatId: message.chatId, command }, 'unknown telegram command, answering as text')
    }

    const thread = last.message_thread_id
    await api.sendChatAction(last.chat.id, 'typing', thread).catch(() => undefined)

    // The conversation so far, as much of it as the budget holds. Read fails
    // soft: a reply without the room's history is worse, a person left
    // unanswered because Mongo hiccupped is worse still.
    const history = await this.transcript
      .recent(message.chatId, {
        maxTokens: this.options.historyTokens ?? 5_000,
        excluding: ids,
      })
      .catch((error: unknown) => {
        logger.warn({ err: error, chatId: message.chatId }, 'could not read the chat transcript')
        return []
      })

    const draft = await this.options.harness.generate(message, history)

    const styled = draft.text
      ? applyHouseStyle(draft.text, { maxEmoji: this.options.maxEmoji ?? 1, maxLength: 1_200 })
      : undefined
    if (styled && styled.fixes.length > 0) {
      logger.info({ fixes: styled.fixes }, 'house style applied to a telegram draft')
    }
    const pieces = styled ? splitForTelegram(styled.text) : []
    if (pieces.length === 0 && !draft.reaction) {
      logger.warn({ chatId: message.chatId }, 'telegram draft had neither text nor a reaction')
      return
    }

    const sendOptions = {
      // Threaded in a room so it is clear who is being answered; a DM reads
      // better without the quote.
      ...(message.chatKind === 'group' ? { replyToMessageId: last.message_id } : {}),
      ...(thread ? { messageThreadId: thread } : {}),
    }
    const said = pieces.length > 0 ? pieces.join('\n') : `(reacted ${draft.reaction ?? ''})`

    if (this.options.dryRun) {
      logger.info(
        {
          chatId: message.chatId,
          text: pieces.join('\n') || undefined,
          reaction: draft.reaction,
          gifQuery: draft.gifQuery,
        },
        'DRY RUN: would have answered on telegram',
      )
      await this.keepOwnTurn(message.chatId, `dry-run:${lastMessage.id}`, said)
      await this.options.harness.remember(message, said, undefined, draft.incomingTone)
      return
    }

    // The reaction lands first: it is instant, and it tells the person they
    // were heard while the text is still on its way.
    if (draft.reaction) {
      await api
        .setMessageReaction(last.chat.id, last.message_id, draft.reaction)
        .catch((error: unknown) =>
          logger.warn({ err: error, reaction: draft.reaction }, 'could not react on telegram'),
        )
    }

    let sentId: number | undefined
    for (const piece of pieces) {
      const sent = await api.sendMessage(last.chat.id, piece, sendOptions)
      sentId ??= sent.id
    }
    await this.keepOwnTurn(
      message.chatId,
      sentId === undefined ? `reaction:${lastMessage.id}` : String(sentId),
      said,
    )
    logger.info(
      {
        chatId: message.chatId,
        kind: message.chatKind,
        answered: ids,
        sentId,
        reaction: draft.reaction,
        tone: draft.incomingTone,
      },
      'answered on telegram',
    )

    // A GIF is a nice touch rather than the point, so it fails soft, and it
    // is rationed: one per chat per cooldown however keen the drafter is.
    if (draft.gifQuery && this.options.gifs && pieces.length > 0) {
      if (!this.gifAllowed(message.chatId)) {
        logger.info({ chatId: message.chatId }, 'gif skipped: cooldown')
      } else {
        try {
          const gifs = this.options.gifs
          const [own] = await gifs.search(draft.gifQuery, { channelOnly: true })
          const [gif] = own ? [own] : await gifs.search(draft.gifQuery, { channelOnly: false })
          if (gif) {
            await api.sendAnimation(last.chat.id, gif.mediaUrl, thread ? { messageThreadId: thread } : {})
            this.lastGifAt.set(message.chatId, this.now())
          }
        } catch (error) {
          logger.warn({ err: error, gifQuery: draft.gifQuery }, 'could not attach a telegram gif')
        }
      }
    }

    await this.options.harness
      .remember(
        message,
        said,
        sentId === undefined ? `reaction:${lastMessage.id}` : String(sentId),
        draft.incomingTone,
      )
      .catch((error: unknown) => logger.warn({ err: error }, 'could not capture a telegram reply'))
  }
}
