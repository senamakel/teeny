/**
 * The slice of the Telegram Bot API this service uses, over plain fetch.
 *
 * Long polling rather than a webhook: the bot then needs no public URL, no
 * TLS certificate on Telegram's list, and no route through the host nginx.
 * The cost is that only one process may poll a token at a time -- Telegram
 * answers a second poller with 409 -- which is why the Telegram role is
 * single-homed in compose, the same way the chart job is.
 */

export interface TelegramUser {
  id: number
  is_bot: boolean
  first_name: string
  last_name?: string
  username?: string
}

export interface TelegramChat {
  id: number
  type: 'private' | 'group' | 'supergroup' | 'channel'
  title?: string
  username?: string
}

export interface TelegramMessageEntity {
  type: string
  offset: number
  length: number
}

export interface TelegramMessage {
  message_id: number
  date: number
  chat: TelegramChat
  from?: TelegramUser
  /** Set when the message was posted as a channel or an anonymous admin, not a person. */
  sender_chat?: TelegramChat
  /** A post from a linked channel, copied into the discussion group by Telegram. */
  is_automatic_forward?: boolean
  /** The inline bot a person sent this through, e.g. a GIF picked via @gif. */
  via_bot?: TelegramUser
  text?: string
  caption?: string
  entities?: TelegramMessageEntity[]
  reply_to_message?: TelegramMessage
  /** Present in forum supergroups; replies must carry it back or land in General. */
  message_thread_id?: number
}

export interface TelegramUpdate {
  update_id: number
  message?: TelegramMessage
}

export interface SendMessageOptions {
  replyToMessageId?: number
  messageThreadId?: number
}

/** The narrow surface the bot loop and the tests depend on. */
export interface TelegramApi {
  getMe(): Promise<TelegramUser>
  getUpdates(options: { offset?: number; timeoutSeconds: number }): Promise<TelegramUpdate[]>
  sendMessage(chatId: number, text: string, options?: SendMessageOptions): Promise<{ id: number }>
  /** A GIF by URL. Telegram fetches it; nothing is uploaded from here. */
  sendAnimation(chatId: number, url: string, options?: SendMessageOptions): Promise<{ id: number }>
  sendChatAction(chatId: number, action: 'typing', messageThreadId?: number): Promise<void>
}

interface TelegramResponse<T> {
  ok: boolean
  result?: T
  description?: string
  error_code?: number
  parameters?: { retry_after?: number }
}

export class TelegramApiError extends Error {
  constructor(
    readonly method: string,
    readonly code: number | undefined,
    description: string,
    readonly retryAfterSeconds?: number,
  ) {
    super(`telegram ${method} failed${code ? ` (${code})` : ''}: ${description}`)
    this.name = 'TelegramApiError'
  }
}

/** Telegram refuses anything over 4096 characters; the reply is split before it gets there. */
export const TELEGRAM_MAX_MESSAGE_LENGTH = 4096

export class TelegramApiClient implements TelegramApi {
  private readonly base: string

  constructor(
    private readonly options: { token: string; fetch?: typeof fetch; apiUrl?: string },
  ) {
    this.base = `${options.apiUrl ?? 'https://api.telegram.org'}/bot${options.token}`
  }

  private async call<T>(
    method: string,
    body: Record<string, unknown>,
    init: { timeoutMs?: number } = {},
  ): Promise<T> {
    const doFetch = this.options.fetch ?? fetch
    const response = await doFetch(`${this.base}/${method}`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(body),
      ...(init.timeoutMs ? { signal: AbortSignal.timeout(init.timeoutMs) } : {}),
    })
    let payload: TelegramResponse<T>
    try {
      payload = (await response.json()) as TelegramResponse<T>
    } catch {
      throw new TelegramApiError(method, response.status, `non-JSON response (${response.status})`)
    }
    if (!payload.ok || payload.result === undefined) {
      throw new TelegramApiError(
        method,
        payload.error_code ?? response.status,
        payload.description ?? 'unknown error',
        payload.parameters?.retry_after,
      )
    }
    return payload.result
  }

  getMe(): Promise<TelegramUser> {
    return this.call<TelegramUser>('getMe', {})
  }

  getUpdates(options: { offset?: number; timeoutSeconds: number }): Promise<TelegramUpdate[]> {
    return this.call<TelegramUpdate[]>(
      'getUpdates',
      {
        ...(options.offset === undefined ? {} : { offset: options.offset }),
        timeout: options.timeoutSeconds,
        // Only messages. Edits, reactions, and channel posts are not
        // conversations the account takes part in.
        allowed_updates: ['message'],
      },
      // The long poll holds the line for `timeout` seconds by design; the
      // client-side limit only has to catch a hung connection past that.
      { timeoutMs: (options.timeoutSeconds + 15) * 1_000 },
    )
  }

  async sendMessage(
    chatId: number,
    text: string,
    options: SendMessageOptions = {},
  ): Promise<{ id: number }> {
    const sent = await this.call<TelegramMessage>('sendMessage', {
      chat_id: chatId,
      text,
      // Plain text on purpose. Markdown mode rejects the whole message when
      // the model leaves a stray underscore or asterisk in it, and a reply
      // that fails to send over formatting helps nobody.
      ...(options.replyToMessageId ? { reply_to_message_id: options.replyToMessageId } : {}),
      ...(options.messageThreadId ? { message_thread_id: options.messageThreadId } : {}),
      link_preview_options: { is_disabled: true },
    })
    return { id: sent.message_id }
  }

  async sendAnimation(
    chatId: number,
    url: string,
    options: SendMessageOptions = {},
  ): Promise<{ id: number }> {
    const sent = await this.call<TelegramMessage>('sendAnimation', {
      chat_id: chatId,
      animation: url,
      ...(options.replyToMessageId ? { reply_to_message_id: options.replyToMessageId } : {}),
      ...(options.messageThreadId ? { message_thread_id: options.messageThreadId } : {}),
    })
    return { id: sent.message_id }
  }

  async sendChatAction(chatId: number, action: 'typing', messageThreadId?: number): Promise<void> {
    await this.call<boolean>('sendChatAction', {
      chat_id: chatId,
      action,
      ...(messageThreadId ? { message_thread_id: messageThreadId } : {}),
    })
  }
}

/**
 * Splits a long reply at paragraph, then line, then word boundaries so each
 * piece fits one Telegram message. A reply that long is rare and probably a
 * mistake, but truncating it mid-sentence is worse than sending two.
 */
export function splitForTelegram(
  text: string,
  limit: number = TELEGRAM_MAX_MESSAGE_LENGTH,
): string[] {
  const trimmed = text.trim()
  if (trimmed.length <= limit) return trimmed ? [trimmed] : []

  const pieces: string[] = []
  let remaining = trimmed
  while (remaining.length > limit) {
    const window = remaining.slice(0, limit)
    const cut = [window.lastIndexOf('\n\n'), window.lastIndexOf('\n'), window.lastIndexOf(' ')]
      .find((index) => index > limit / 4) ?? limit
    pieces.push(remaining.slice(0, cut).trim())
    remaining = remaining.slice(cut).trim()
  }
  if (remaining) pieces.push(remaining)
  return pieces
}
