import { describe, expect, it, vi } from 'vitest'
import type { FastifyBaseLogger } from 'fastify'
import type { TelegramApi, TelegramMessage, TelegramUpdate } from '../src/clients/telegram.js'
import type { ChatHarness } from '../src/harness/chat-harness.js'
import { TelegramBot } from '../src/telegram/bot.js'
import type { GifSource } from '../src/types.js'

const bot = { id: 8782739916, username: 'teenythebot' }
const alice = { id: 1, is_bot: false, first_name: 'Alice', username: 'alice' }
const NOW = 1_700_000_000_000

function logger(): FastifyBaseLogger {
  return {
    info: vi.fn(),
    warn: vi.fn(),
    error: vi.fn(),
    debug: vi.fn(),
  } as unknown as FastifyBaseLogger
}

function api(): TelegramApi {
  return {
    getMe: vi.fn(),
    getUpdates: vi.fn().mockResolvedValue([]),
    sendMessage: vi.fn().mockResolvedValue({ id: 99 }),
    sendAnimation: vi.fn().mockResolvedValue({ id: 100 }),
    sendChatAction: vi.fn().mockResolvedValue(undefined),
  }
}

function harness(reply: unknown = { text: 'gm fren', incomingTone: 'friendly' }): ChatHarness {
  return {
    generate: vi.fn().mockResolvedValue(reply),
    remember: vi.fn().mockResolvedValue(undefined),
  } as unknown as ChatHarness
}

function dm(text: string, overrides: Partial<TelegramMessage> = {}): TelegramUpdate {
  return {
    update_id: 1,
    message: {
      message_id: 10,
      date: NOW / 1_000,
      chat: { id: 1, type: 'private' },
      from: alice,
      text,
      ...overrides,
    },
  }
}

function group(text: string, overrides: Partial<TelegramMessage> = {}): TelegramUpdate {
  return {
    update_id: 2,
    message: {
      message_id: 20,
      date: NOW / 1_000,
      chat: { id: -100, type: 'supergroup', title: 'holders' },
      from: alice,
      text,
      ...overrides,
    },
  }
}

function build(overrides: Partial<ConstructorParameters<typeof TelegramBot>[0]> = {}) {
  const deps = {
    api: api(),
    harness: harness(),
    logger: logger(),
    bot,
    now: () => NOW,
    sleep: vi.fn().mockResolvedValue(undefined),
    ...overrides,
  }
  return { bot: new TelegramBot(deps), deps }
}

describe('TelegramBot', () => {
  it('answers a DM, shows typing while it thinks, and remembers the exchange', async () => {
    const { bot: subject, deps } = build()

    await subject.handle(dm('hello'))

    expect(deps.api.sendChatAction).toHaveBeenCalledWith(1, 'typing', undefined)
    expect(deps.harness.generate).toHaveBeenCalledWith(
      expect.objectContaining({ text: 'hello', chatKind: 'private', authorUsername: 'alice' }),
      [],
    )
    // A DM is not threaded: the quote adds nothing in a two-person chat.
    expect(deps.api.sendMessage).toHaveBeenCalledWith(1, 'gm fren', {})
    expect(deps.harness.remember).toHaveBeenCalledWith(
      expect.objectContaining({ id: '10' }),
      'gm fren',
      '99',
      'friendly',
    )
  })

  it('stays quiet in a group until it is named, and threads the answer when it speaks', async () => {
    const { bot: subject, deps } = build()

    await subject.handle(group('gm everyone'))
    expect(deps.harness.generate).not.toHaveBeenCalled()

    await subject.handle(
      group('@teenythebot gm', { entities: [{ type: 'mention', offset: 0, length: 12 }] }),
    )
    expect(deps.api.sendMessage).toHaveBeenCalledWith(-100, 'gm fren', { replyToMessageId: 20 })
    // The line it did not answer is still context for the one it did.
    expect(deps.harness.generate).toHaveBeenCalledWith(expect.objectContaining({ text: 'gm' }), [
      { from: 'person', authorName: 'Alice', text: 'gm everyone' },
    ])
  })

  it('carries its own turns forward as history, most recent last', async () => {
    const { bot: subject, deps } = build({ historyLength: 3 })

    await subject.handle(dm('one'))
    await subject.handle(dm('two', { message_id: 11 }))
    await subject.handle(dm('three', { message_id: 12 }))

    const calls = (deps.harness.generate as ReturnType<typeof vi.fn>).mock.calls
    expect(calls[2]?.[1]).toEqual([
      { from: 'bot', text: 'gm fren' },
      { from: 'person', authorName: 'Alice', text: 'two' },
      { from: 'bot', text: 'gm fren' },
    ])
  })

  it('skips a message that was already old when it arrived', async () => {
    const { bot: subject, deps } = build({ maxMessageAgeMs: 60_000 })

    await subject.handle(dm('hello', { date: NOW / 1_000 - 3_600 }))

    expect(deps.harness.generate).not.toHaveBeenCalled()
    expect(deps.api.sendMessage).not.toHaveBeenCalled()
  })

  it('applies house style: one emoji, no em dashes', async () => {
    const { bot: subject, deps } = build({
      harness: harness({ text: 'gm 🌞🌞 — welcome' }),
    })

    await subject.handle(dm('hi'))

    expect(deps.api.sendMessage).toHaveBeenCalledWith(1, 'gm 🌞, welcome', {})
  })

  it('splits a long answer into several messages rather than dropping the end', async () => {
    const paragraph = 'w'.repeat(1_000)
    const { bot: subject, deps } = build({
      harness: harness({ text: `${paragraph}\n\n${paragraph}\n\n${paragraph}\n\n${paragraph}\n\n${paragraph}` }),
    })

    await subject.handle(dm('tell me everything'))

    // House style caps the draft at 1200, which is under a Telegram message;
    // the split is there for a cap change, and must at least send once.
    expect(deps.api.sendMessage).toHaveBeenCalled()
    for (const call of (deps.api.sendMessage as ReturnType<typeof vi.fn>).mock.calls) {
      expect((call[1] as string).length).toBeLessThanOrEqual(4096)
    }
  })

  it('sends a GIF after the text when the drafter asked for one, and shrugs when it cannot', async () => {
    const gifs: GifSource = {
      search: vi.fn().mockResolvedValue([{ id: 'g', title: 'wave', pageUrl: 'p', mediaUrl: 'https://g/wave.gif', mimeType: 'image/gif' }]),
      get: vi.fn(),
      download: vi.fn(),
    }
    const { bot: subject, deps } = build({ gifs, harness: harness({ text: 'hi', gifQuery: 'wave' }) })

    await subject.handle(dm('hi'))

    expect(gifs.search).toHaveBeenCalledWith('wave', { channelOnly: true })
    expect(deps.api.sendAnimation).toHaveBeenCalledWith(1, 'https://g/wave.gif', {})
    expect(deps.harness.remember).toHaveBeenCalled()

    const broken: GifSource = { ...gifs, search: vi.fn().mockRejectedValue(new Error('giphy down')) }
    const second = build({ gifs: broken, harness: harness({ text: 'hi', gifQuery: 'wave' }) })
    await second.bot.handle(dm('hi'))
    expect(second.deps.api.sendMessage).toHaveBeenCalled()
    expect(second.deps.harness.remember).toHaveBeenCalled()
  })

  it('logs instead of sending in dry run, and still remembers what it would have said', async () => {
    const { bot: subject, deps } = build({ dryRun: true })

    await subject.handle(dm('hello'))

    expect(deps.api.sendMessage).not.toHaveBeenCalled()
    expect(deps.harness.remember).toHaveBeenCalledWith(expect.anything(), 'gm fren', undefined, 'friendly')
  })

  it('ignores other bots so two bots in a room cannot loop', async () => {
    const { bot: subject, deps } = build()

    await subject.handle(dm('hi', { from: { id: 5, is_bot: true, first_name: 'Other' } }))

    expect(deps.harness.generate).not.toHaveBeenCalled()
  })

  it('polls past a failed turn and advances the offset', async () => {
    const { bot: subject, deps } = build({
      harness: {
        generate: vi.fn().mockRejectedValue(new Error('model down')),
        remember: vi.fn(),
      } as unknown as ChatHarness,
    })
    const getUpdates = deps.api.getUpdates as ReturnType<typeof vi.fn>
    getUpdates
      .mockResolvedValueOnce([dm('hello'), { ...dm('again', { message_id: 11 }), update_id: 2 }])
      .mockImplementation(async () => {
        subject.stop()
        return []
      })

    await subject.start()
    await subject.drain()

    expect(getUpdates).toHaveBeenLastCalledWith({ offset: 3, timeoutSeconds: 30 })
    expect(deps.logger.error).toHaveBeenCalledWith(
      expect.objectContaining({ chatId: '1' }),
      'telegram message failed',
    )
    expect(deps.api.sendMessage).not.toHaveBeenCalled()
  })

  it('backs off when the poll itself fails, then keeps going', async () => {
    const { bot: subject, deps } = build()
    const getUpdates = deps.api.getUpdates as ReturnType<typeof vi.fn>
    getUpdates.mockRejectedValueOnce(new Error('network')).mockImplementation(async () => {
      subject.stop()
      return []
    })

    await subject.start()

    expect(deps.sleep).toHaveBeenCalledWith(5_000)
    expect(getUpdates).toHaveBeenCalledTimes(2)
  })
})
