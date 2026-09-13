import type { FastifyBaseLogger } from 'fastify'
import { applyHouseStyle } from '../agent/house-style.js'
import {
  TelegramApiError,
  splitForTelegram,
  type TelegramApi,
  type TelegramUpdate,
} from '../clients/telegram.js'
import type { ChatHarness } from '../harness/chat-harness.js'
import type { ChatTurn, GifSource } from '../types.js'
import { commandOf, isAddressedToBot, toChatMessage, type BotIdentity } from './inbox.js'

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
  /** How many turns of a chat the drafter is shown. */
  historyLength?: number
  /** Injectable for the tests. */
  sleep?: (ms: number) => Promise<void>
  now?: () => number
}

const defaultSleep = (ms: number): Promise<void> =>
  new Promise((resolve) => setTimeout(resolve, ms))

/**
 * The Telegram side of the account: one long-poll loop, one drafter per turn.
 *
 * Chats are handled one message at a time each, and different chats in
 * parallel. A DM waiting on a slow group is a person left on read; two
 * replies to the same person racing each other is a conversation that reads
 * out of order. Both are avoided by serialising per chat and nowhere else.
 */
export class TelegramBot {
  private offset: number | undefined
  private stopped = false
  private readonly chains = new Map<string, Promise<void>>()
  private readonly histories = new Map<string, ChatTurn[]>()
  private readonly sleep: (ms: number) => Promise<void>
  private readonly now: () => number

  constructor(private readonly options: TelegramBotOptions) {
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
          this.enqueue(update)
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
    await Promise.all(this.chains.values())
  }

  private enqueue(update: TelegramUpdate): void {
    const message = update.message
    if (!message) return
    const key = String(message.chat.id)
    const previous = this.chains.get(key) ?? Promise.resolve()
    const next = previous
      .then(() => this.handle(update))
      .catch((error: unknown) => {
        this.options.logger.error(
          { err: error, chatId: key, messageId: message.message_id },
          'telegram message failed',
        )
      })
    this.chains.set(key, next)
    void next.finally(() => {
      if (this.chains.get(key) === next) this.chains.delete(key)
    })
  }

  private history(chatId: string): ChatTurn[] {
    return this.histories.get(chatId) ?? []
  }

  private pushHistory(chatId: string, turn: ChatTurn): void {
    const limit = this.options.historyLength ?? 12
    const turns = [...this.history(chatId), turn].slice(-limit)
    this.histories.set(chatId, turns)
  }

  /** One update, start to finish. Public so the tests can drive it directly. */
  async handle(update: TelegramUpdate): Promise<void> {
    const raw = update.message
    if (!raw) return
    const { logger, bot, api } = this.options
    const message = toChatMessage(raw, bot)
    if (!message) return

    // Seen is not the same as answered: a line in a group that was not for the
    // account still tells the next reply what the room was talking about.
    const history = this.history(message.chatId)
    this.pushHistory(message.chatId, {
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

    // A command other than /start is not something the account does. It is
    // answered as a message anyway, so a typo does not go unanswered.
    const command = commandOf(raw)
    if (command && command !== 'start') {
      logger.info({ chatId: message.chatId, command }, 'unknown telegram command, answering as text')
    }

    const thread = raw.message_thread_id
    await api.sendChatAction(raw.chat.id, 'typing', thread).catch(() => undefined)

    const draft = await this.options.harness.generate(message, history)
    const styled = applyHouseStyle(draft.text, {
      maxEmoji: this.options.maxEmoji ?? 1,
      maxLength: 1_200,
    })
    if (styled.fixes.length > 0) {
      logger.info({ fixes: styled.fixes }, 'house style applied to a telegram draft')
    }
    const pieces = splitForTelegram(styled.text)
    if (pieces.length === 0) {
      logger.warn({ chatId: message.chatId }, 'telegram draft was empty after house style')
      return
    }

    const sendOptions = {
      // Threaded in a room so it is clear who is being answered; a DM reads
      // better without the quote.
      ...(message.chatKind === 'group' ? { replyToMessageId: raw.message_id } : {}),
      ...(thread ? { messageThreadId: thread } : {}),
    }

    if (this.options.dryRun) {
      logger.info(
        { chatId: message.chatId, text: styled.text, gifQuery: draft.gifQuery },
        'DRY RUN: would have sent a telegram message',
      )
      this.pushHistory(message.chatId, { from: 'bot', text: styled.text })
      await this.options.harness.remember(message, styled.text, undefined, draft.incomingTone)
      return
    }

    let sentId: number | undefined
    for (const piece of pieces) {
      const sent = await api.sendMessage(raw.chat.id, piece, sendOptions)
      sentId ??= sent.id
    }
    this.pushHistory(message.chatId, { from: 'bot', text: styled.text })
    logger.info(
      { chatId: message.chatId, kind: message.chatKind, sentId, tone: draft.incomingTone },
      'sent a telegram message',
    )

    // A GIF is a nice touch rather than the point, so it fails soft.
    if (draft.gifQuery && this.options.gifs) {
      try {
        const gifs = this.options.gifs
        const [own] = await gifs.search(draft.gifQuery, { channelOnly: true })
        const [gif] = own ? [own] : await gifs.search(draft.gifQuery, { channelOnly: false })
        if (gif) {
          await api.sendAnimation(raw.chat.id, gif.mediaUrl, thread ? { messageThreadId: thread } : {})
        }
      } catch (error) {
        logger.warn({ err: error, gifQuery: draft.gifQuery }, 'could not attach a telegram gif')
      }
    }

    await this.options.harness
      .remember(message, styled.text, String(sentId), draft.incomingTone)
      .catch((error: unknown) => logger.warn({ err: error }, 'could not capture a telegram reply'))
  }
}
