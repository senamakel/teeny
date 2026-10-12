import { describe, expect, it } from 'vitest'
import type { TelegramMessage } from '../src/clients/telegram.js'
import {
  commandOf,
  isAddressedToBot,
  isFromBot,
  isServiceMessage,
  stripBotMention,
  toChatMessage,
} from '../src/telegram/inbox.js'

const bot = { id: 8782739916, username: 'teenythebot' }
const alice = { id: 1, is_bot: false, first_name: 'Alice', last_name: 'Liddell', username: 'alice' }

function message(overrides: Partial<TelegramMessage> = {}): TelegramMessage {
  return {
    message_id: 10,
    date: 1_700_000_000,
    chat: { id: -100, type: 'supergroup', title: 'TINY holders' },
    from: alice,
    text: 'hello',
    ...overrides,
  }
}

/** A message with no text at all, as a photo without a caption has. */
function textless(overrides: Partial<TelegramMessage> = {}): TelegramMessage {
  const { text: _text, ...rest } = message(overrides)
  return rest
}

describe('isAddressedToBot', () => {
  it('answers everything in a DM', () => {
    expect(isAddressedToBot(message({ chat: { id: 1, type: 'private' } }), bot)).toBe(true)
  })

  it('stays quiet in a group unless named', () => {
    expect(isAddressedToBot(message({ text: 'gm everyone' }), bot)).toBe(false)
  })

  it('answers a group message that mentions it by handle', () => {
    const text = 'hey @teenythebot how is $TINY doing'
    const entities = [{ type: 'mention', offset: 4, length: '@teenythebot'.length }]
    expect(isAddressedToBot(message({ text, entities }), bot)).toBe(true)
  })

  it('is not fooled by a mention of some other bot', () => {
    const text = 'hey @otherbot how is $TINY doing'
    const entities = [{ type: 'mention', offset: 4, length: '@otherbot'.length }]
    expect(isAddressedToBot(message({ text, entities }), bot)).toBe(false)
  })

  it('answers a reply to something it said', () => {
    const mine: TelegramMessage = {
      message_id: 9,
      date: 1_700_000_000,
      chat: { id: -100, type: 'supergroup' },
      from: { id: bot.id, is_bot: true, first_name: 'Teeny', username: bot.username },
      text: 'gm',
    }
    expect(isAddressedToBot(message({ text: 'gm back', reply_to_message: mine }), bot)).toBe(true)
  })

  it('answers a command addressed to it, even without an entity list', () => {
    expect(isAddressedToBot(message({ text: '/start@teenythebot' }), bot)).toBe(true)
    expect(isAddressedToBot(message({ text: '/start@otherbot' }), bot)).toBe(false)
  })
})

describe('stripBotMention', () => {
  it('removes the handle so the drafter sees the question, not the tag', () => {
    expect(stripBotMention('@teenythebot what is $TINY?', bot)).toBe('what is $TINY?')
    expect(stripBotMention('is @TeenyTheBot awake', bot)).toBe('is awake')
    expect(stripBotMention('/start@teenythebot', bot)).toBe('/start')
  })
})

describe('commandOf', () => {
  it('reads the command with or without the bot suffix', () => {
    expect(commandOf(message({ text: '/start' }))).toBe('start')
    expect(commandOf(message({ text: '/Start@teenythebot hi' }))).toBe('start')
    expect(commandOf(message({ text: 'hello /start' }))).toBeUndefined()
  })
})

describe('toChatMessage', () => {
  it('normalises a group message', () => {
    const text = '@teenythebot gm'
    expect(
      toChatMessage(
        message({ text, entities: [{ type: 'mention', offset: 0, length: 12 }] }),
        bot,
      ),
    ).toEqual({
      id: '10',
      channel: 'telegram',
      chatId: '-100',
      chatKind: 'group',
      chatTitle: 'TINY holders',
      text: 'gm',
      authorId: '1',
      authorUsername: 'alice',
      authorName: 'Alice Liddell',
      createdAt: '2023-11-14T22:13:20.000Z',
    })
  })

  it('marks a DM as private', () => {
    expect(toChatMessage(message({ chat: { id: 1, type: 'private' } }), bot)?.chatKind).toBe(
      'private',
    )
  })

  it('ignores other bots, itself, and messages with no text', () => {
    expect(
      toChatMessage(message({ from: { id: 5, is_bot: true, first_name: 'Other' } }), bot),
    ).toBeUndefined()
    expect(
      toChatMessage(message({ from: { id: bot.id, is_bot: true, first_name: 'Teeny' } }), bot),
    ).toBeUndefined()
    expect(toChatMessage(textless(), bot)).toBeUndefined()
    expect(toChatMessage(message({ text: '@teenythebot' }), bot)).toBeUndefined()
  })

  it('keeps a photo with no caption, at its largest size, and an image sent as a file', () => {
    const photo = [
      { file_id: 'thumb', file_unique_id: 't', width: 90, height: 90 },
      { file_id: 'full', file_unique_id: 'f', width: 1280, height: 960 },
    ]
    expect(toChatMessage(textless({ photo }), bot)).toMatchObject({
      text: '',
      attachments: [{ kind: 'image', ref: 'full', mimeType: 'image/jpeg' }],
    })
    expect(toChatMessage(textless({ caption: 'look', photo }), bot)?.text).toBe('look')
    expect(
      toChatMessage(
        textless({ document: { file_id: 'doc', file_unique_id: 'd', mime_type: 'image/png', file_name: 'a.png' } }),
        bot,
      )?.attachments,
    ).toEqual([{ kind: 'image', ref: 'doc', mimeType: 'image/png' }])
    // A PDF is not a picture, and a message that is only a PDF is not a line.
    expect(
      toChatMessage(
        textless({ document: { file_id: 'doc', file_unique_id: 'd', mime_type: 'application/pdf' } }),
        bot,
      ),
    ).toBeUndefined()
  })

  /**
   * A buy bot's alert, a channel post, an anonymous admin: none of them is a
   * person, and answering any of them is noise at best and a loop at worst.
   */
  it('ignores anything that came from software rather than a person', () => {
    const buyBot = { id: 5, is_bot: true, first_name: 'Buy Bot', username: 'somebuybot' }
    const placeholder = { id: 136817688, is_bot: true, first_name: 'Channel', username: 'Channel_Bot' }
    const channel = { id: -1001, type: 'channel' as const, title: 'TINY news' }

    expect(isFromBot(message({ from: buyBot }))).toBe(true)
    expect(isFromBot(message({ from: placeholder, sender_chat: channel }))).toBe(true)
    expect(isFromBot(message({ sender_chat: { id: -100, type: 'supergroup' } }))).toBe(true)
    expect(isFromBot(message({ is_automatic_forward: true }))).toBe(true)
    expect(isFromBot(message())).toBe(false)
    // A person picking a GIF through @gif is still a person.
    expect(isFromBot(message({ via_bot: { id: 7, is_bot: true, first_name: 'gif' } }))).toBe(false)

    expect(toChatMessage(message({ from: buyBot, text: '@teenythebot 🟢 BUY 12 SOL' }), bot)).toBeUndefined()
    expect(toChatMessage(message({ from: placeholder, sender_chat: channel }), bot)).toBeUndefined()
    expect(toChatMessage(message({ is_automatic_forward: true }), bot)).toBeUndefined()
  })

  it('drops service messages: joins, leaves, pins, title changes', () => {
    const join = message({ text: undefined as unknown as string, new_chat_members: [alice] })
    expect(isServiceMessage(join)).toBe(true)
    expect(toChatMessage(join, bot)).toBeUndefined()
    expect(toChatMessage(message({ pinned_message: {}, text: 'pinned' }), bot)).toBeUndefined()
    expect(toChatMessage(message({ new_chat_title: 'TINY fam' }), bot)).toBeUndefined()
    expect(toChatMessage(message({ left_chat_member: alice }), bot)).toBeUndefined()
    expect(isServiceMessage(message())).toBe(false)
  })

  it('reads a caption when a photo carries the question', () => {
    expect(toChatMessage(textless({ caption: 'what is this chart' }), bot)?.text).toBe(
      'what is this chart',
    )
  })
})
