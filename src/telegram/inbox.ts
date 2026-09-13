import type { TelegramMessage, TelegramUser } from '../clients/telegram.js'
import type { ChatMessage } from '../types.js'

export interface BotIdentity {
  id: number
  username: string
}

/**
 * Whether a message in a room is for the account.
 *
 * In a DM everything is. In a group the account is one voice among many, and
 * answering every line is the fastest way to get a bot kicked: it speaks
 * when named, when someone replies to something it said, or when it is given
 * a command. Telegram's privacy mode already withholds the rest from the bot;
 * this is the same rule applied again on this side, so an admin turning
 * privacy mode off does not turn the account into a chatterbox.
 */
export function isAddressedToBot(message: TelegramMessage, bot: BotIdentity): boolean {
  if (message.chat.type === 'private') return true
  if (message.reply_to_message?.from?.id === bot.id) return true
  const text = message.text ?? message.caption ?? ''
  const handle = bot.username.toLowerCase()
  for (const entity of message.entities ?? []) {
    if (entity.type !== 'mention') continue
    const mention = text.slice(entity.offset, entity.offset + entity.length).toLowerCase()
    if (mention === `@${handle}`) return true
  }
  // `/start@teenythebot` is addressed even when the entity list is missing.
  return new RegExp(`^/\\w+@${bot.username}\\b`, 'i').test(text)
}

/**
 * The account's own handle removed from the text, so the drafter sees "what is
 * $TINY?" rather than "@teenythebot what is $TINY?" and does not read the
 * mention as part of the question.
 */
export function stripBotMention(text: string, bot: BotIdentity): string {
  return text
    .replace(new RegExp(`@${bot.username}\\b`, 'gi'), '')
    .replace(new RegExp(`^(/\\w+)@${bot.username}\\b`, 'i'), '$1')
    .replace(/[ \t]{2,}/g, ' ')
    .trim()
}

/** The command a message starts with, without the bot suffix, or undefined. */
export function commandOf(message: TelegramMessage): string | undefined {
  const text = message.text ?? ''
  const match = /^\/([a-z0-9_]+)(?:@\w+)?(?:\s|$)/i.exec(text)
  return match?.[1]?.toLowerCase()
}

function displayName(user: TelegramUser): string {
  return [user.first_name, user.last_name].filter(Boolean).join(' ')
}

/**
 * Whether a message came from software rather than a person.
 *
 * Bots do not talk to bots here, which closes the loop two bots in one group
 * would otherwise fall into, and keeps the account from answering a buy bot's
 * every alert. Three shapes count: a bot user; a message posted as a channel
 * or an anonymous admin, where `from` is Telegram's placeholder and the real
 * sender is a chat; and a channel post Telegram copied into the linked
 * discussion group. A person sending a GIF through an inline bot is still a
 * person, so `via_bot` does not count.
 */
export function isFromBot(message: TelegramMessage): boolean {
  if (!message.from || message.from.is_bot) return true
  if (message.sender_chat) return true
  return message.is_automatic_forward === true
}

/**
 * A Telegram message as the rest of the service sees it, or nothing when the
 * message is not something to answer: no text, from a bot, or from the
 * account itself.
 */
export function toChatMessage(
  message: TelegramMessage,
  bot: BotIdentity,
): ChatMessage | undefined {
  const author = message.from
  if (!author || isFromBot(message) || author.id === bot.id) return undefined
  const raw = message.text ?? message.caption
  if (!raw) return undefined
  const text = stripBotMention(raw, bot)
  if (!text) return undefined

  return {
    id: String(message.message_id),
    channel: 'telegram',
    chatId: String(message.chat.id),
    chatKind: message.chat.type === 'private' ? 'private' : 'group',
    ...(message.chat.title ? { chatTitle: message.chat.title } : {}),
    text,
    authorId: String(author.id),
    ...(author.username ? { authorUsername: author.username } : {}),
    authorName: displayName(author),
    createdAt: new Date(message.date * 1_000).toISOString(),
    ...(message.reply_to_message?.from?.id === bot.id ? { repliesToBot: true } : {}),
  }
}
