import { describe, expect, it } from 'vitest'
import type { TelegramMessage } from '../src/clients/telegram.js'
import { commandOf, isAddressedToBot, stripBotMention, toChatMessage } from '../src/telegram/inbox.js'

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

  it('reads a caption when a photo carries the question', () => {
    expect(toChatMessage(textless({ caption: 'what is this chart' }), bot)?.text).toBe(
      'what is this chart',
    )
  })
})
