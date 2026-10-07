import { describe, expect, it, vi } from 'vitest'
import type { FastifyBaseLogger } from 'fastify'
import type { TelegramApi, TelegramMessage, TelegramUpdate } from '../src/clients/telegram.js'
import type { ChatHarness } from '../src/harness/chat-harness.js'
import { MediaStore } from '../src/media/store.js'
import type { ImageReader } from '../src/media/types.js'
import { lexicalGate } from '../src/safety/gate.js'
import { TelegramBot } from '../src/telegram/bot.js'
import { MemoryChatTranscript } from '../src/telegram/transcript.js'
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

let sent = 99
function api(): TelegramApi {
  return {
    getMe: vi.fn(),
    getUpdates: vi.fn().mockResolvedValue([]),
    // Each send gets its own id, as Telegram hands out, so the transcript's
    // redelivery guard does not mistake two replies for one.
    sendMessage: vi.fn().mockImplementation(async () => ({ id: (sent += 1) })),
    sendAnimation: vi.fn().mockResolvedValue({ id: 100 }),
    sendChatAction: vi.fn().mockResolvedValue(undefined),
    setMessageReaction: vi.fn().mockResolvedValue(undefined),
    sendPhoto: vi.fn().mockResolvedValue({ id: 200 }),
    sendVideo: vi.fn().mockResolvedValue({ id: 201 }),
    downloadFile: vi.fn().mockResolvedValue({ data: new Uint8Array([1, 2, 3]), mimeType: 'image/jpeg' }),
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

/** A photo message: no `text` at all, a caption at most, as Telegram sends one. */
function photoMessage(
  shape: (text: string, overrides?: Partial<TelegramMessage>) => TelegramUpdate,
  overrides: Partial<TelegramMessage>,
): TelegramUpdate {
  const update = shape('placeholder', overrides)
  const { text: _text, ...message } = update.message!
  return { ...update, message }
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
      expect.stringMatching(/^\d+$/),
      'friendly',
    )
  })

  it('stays quiet in a group until it is named, and threads the answer when it speaks', async () => {
    const { bot: subject, deps } = build()

    await subject.handle(group('gm everyone'))
    expect(deps.harness.generate).not.toHaveBeenCalled()

    await subject.handle(
      group('@teenythebot gm', {
        message_id: 21,
        entities: [{ type: 'mention', offset: 0, length: 12 }],
      }),
    )
    expect(deps.api.sendMessage).toHaveBeenCalledWith(-100, 'gm fren', { replyToMessageId: 21 })
    // The line it did not answer is still context for the one it did.
    expect(deps.harness.generate).toHaveBeenCalledWith(expect.objectContaining({ text: 'gm' }), [
      { from: 'person', authorName: 'Alice', text: 'gm everyone' },
    ])
  })

  it('shows the drafter the whole conversation so far, its own turns included', async () => {
    const { bot: subject, deps } = build()

    await subject.handle(dm('one'))
    await subject.handle(dm('two', { message_id: 11 }))
    await subject.handle(dm('three', { message_id: 12 }))

    const calls = (deps.harness.generate as ReturnType<typeof vi.fn>).mock.calls
    expect(calls[2]?.[1]).toEqual([
      { from: 'person', authorName: 'Alice', text: 'one' },
      { from: 'bot', text: 'gm fren' },
      { from: 'person', authorName: 'Alice', text: 'two' },
      { from: 'bot', text: 'gm fren' },
    ])
  })

  it('reads the history back within the token budget, most recent kept', async () => {
    const { bot: subject, deps } = build({ historyTokens: 40 })
    for (let i = 1; i <= 6; i += 1) {
      await subject.handle(dm(`message number ${i} with some words in it`, { message_id: 10 + i }))
    }
    const calls = (deps.harness.generate as ReturnType<typeof vi.fn>).mock.calls
    const shown = calls[5]?.[1] as Array<{ text: string }>
    expect(shown.length).toBeGreaterThan(0)
    expect(shown.length).toBeLessThan(11)
    expect(shown[shown.length - 1]?.text).toBe('gm fren')
  })

  it('still answers when the transcript store is down', async () => {
    const broken = {
      append: vi.fn().mockRejectedValue(new Error('mongo down')),
      recent: vi.fn().mockRejectedValue(new Error('mongo down')),
      close: vi.fn(),
    }
    const { bot: subject, deps } = build({ transcript: broken })

    await expect(subject.receive(dm('hello'))).rejects.toThrow('mongo down')
    expect(deps.api.sendMessage).not.toHaveBeenCalled()

    const readOnlyBroken = { ...broken, append: vi.fn().mockResolvedValue(undefined) }
    const second = build({ transcript: readOnlyBroken })
    await second.bot.handle(dm('hello'))
    expect(second.deps.api.sendMessage).toHaveBeenCalledWith(1, 'gm fren', {})
    expect(second.deps.harness.generate).toHaveBeenCalledWith(expect.anything(), [])
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

  it('reacts when the drafter asks, before the text goes out', async () => {
    const { bot: subject, deps } = build({ harness: harness({ text: 'ty fren', reaction: '❤' }) })

    await subject.handle(dm('love what you are building'))

    expect(deps.api.setMessageReaction).toHaveBeenCalledWith(1, 10, '❤')
    expect(deps.api.sendMessage).toHaveBeenCalledWith(1, 'ty fren', {})
    const reactOrder = (deps.api.setMessageReaction as ReturnType<typeof vi.fn>).mock.invocationCallOrder[0] ?? 0
    const sendOrder = (deps.api.sendMessage as ReturnType<typeof vi.fn>).mock.invocationCallOrder[0] ?? 0
    expect(reactOrder).toBeLessThan(sendOrder)
  })

  it('can answer with a reaction alone, and remembers that it did', async () => {
    const { bot: subject, deps } = build({ harness: harness({ reaction: '🔥', incomingTone: 'friendly' }) })

    await subject.handle(dm('gm'))
    await subject.handle(dm('and what do you do?', { message_id: 11 }))

    expect(deps.api.setMessageReaction).toHaveBeenCalledWith(1, 10, '🔥')
    expect(deps.api.sendMessage).not.toHaveBeenCalled()
    expect(deps.harness.remember).toHaveBeenCalledWith(expect.anything(), '(reacted 🔥)', 'reaction:10', 'friendly')
    // The next turn can see that it reacted rather than ignored.
    const calls = (deps.harness.generate as ReturnType<typeof vi.fn>).mock.calls
    expect(calls[1]?.[1]).toEqual([
      { from: 'person', authorName: 'Alice', text: 'gm' },
      { from: 'bot', text: '(reacted 🔥)' },
    ])
  })

  it('keeps going when the reaction is refused', async () => {
    const { bot: subject, deps } = build({ harness: harness({ text: 'hi', reaction: '👍' }) })
    ;(deps.api.setMessageReaction as ReturnType<typeof vi.fn>).mockRejectedValue(new Error('REACTION_INVALID'))

    await subject.handle(dm('hey'))

    expect(deps.api.sendMessage).toHaveBeenCalledWith(1, 'hi', {})
  })

  it('rations GIFs to one per chat per cooldown', async () => {
    const gifs: GifSource = {
      search: vi.fn().mockResolvedValue([{ id: 'g', title: 'wave', pageUrl: 'p', mediaUrl: 'https://g/wave.gif', mimeType: 'image/gif' }]),
      get: vi.fn(),
      download: vi.fn(),
    }
    let clock = NOW
    const { bot: subject, deps } = build({
      gifs,
      gifCooldownMs: 60_000,
      now: () => clock,
      harness: harness({ text: 'hi', gifQuery: 'wave' }),
    })

    await subject.handle(dm('hi'))
    await subject.handle(dm('hi again', { message_id: 11 }))
    expect(deps.api.sendAnimation).toHaveBeenCalledTimes(1)

    clock += 61_000
    await subject.handle(dm('hi once more', { message_id: 12 }))
    expect(deps.api.sendAnimation).toHaveBeenCalledTimes(2)
  })

  it('answers a thought typed in pieces as one turn', async () => {
    vi.useFakeTimers({ now: NOW })
    try {
      const { bot: subject, deps } = build({ debounceMs: 2_000 })

      await subject.receive(dm('hey'))
      await subject.receive(dm('quick q', { message_id: 11 }))
      await subject.receive(dm('what does $TINY actually do', { message_id: 12 }))
      expect(deps.harness.generate).not.toHaveBeenCalled()

      await vi.advanceTimersByTimeAsync(2_100)
      await subject.drain()

      expect(deps.harness.generate).toHaveBeenCalledTimes(1)
      expect(deps.harness.generate).toHaveBeenCalledWith(
        expect.objectContaining({ id: '12', text: 'hey\nquick q\nwhat does $TINY actually do' }),
        [],
      )
      expect(deps.api.sendMessage).toHaveBeenCalledTimes(1)
    } finally {
      vi.useRealTimers()
    }
  })

  it('does not batch two different people together', async () => {
    vi.useFakeTimers({ now: NOW })
    try {
      const { bot: subject, deps } = build({ debounceMs: 2_000 })
      const bob = { id: 2, is_bot: false, first_name: 'Bob' }

      await subject.receive(group('@teenythebot gm', { message_id: 21, entities: [{ type: 'mention', offset: 0, length: 12 }] }))
      await subject.receive(group('@teenythebot hello', { message_id: 22, from: bob, entities: [{ type: 'mention', offset: 0, length: 12 }] }))
      await vi.advanceTimersByTimeAsync(2_100)
      await subject.drain()

      expect(deps.harness.generate).toHaveBeenCalledTimes(2)
      expect(deps.api.sendMessage).toHaveBeenCalledWith(-100, 'gm fren', { replyToMessageId: 21 })
      expect(deps.api.sendMessage).toHaveBeenCalledWith(-100, 'gm fren', { replyToMessageId: 22 })
    } finally {
      vi.useRealTimers()
    }
  })

  it('logs instead of sending in dry run, and still remembers what it would have said', async () => {
    const { bot: subject, deps } = build({ dryRun: true })

    await subject.handle(dm('hello'))

    expect(deps.api.sendMessage).not.toHaveBeenCalled()
    expect(deps.harness.remember).toHaveBeenCalledWith(expect.anything(), 'gm fren', undefined, 'friendly')
  })

  describe('pictures', () => {
    const photo = [
      { file_id: 'small', file_unique_id: 's', width: 90, height: 90 },
      { file_id: 'large', file_unique_id: 'l', width: 800, height: 800 },
    ]

    it('looks at a photo sent to it and hands the drafter what it saw', async () => {
      const vision: ImageReader = { describe: vi.fn().mockResolvedValue('a chart of $TINY, up and to the right') }
      const { bot: subject, deps } = build({ vision })

      await subject.handle(photoMessage(dm, { caption: 'thoughts?', photo }))

      // The largest rendition, not the thumbnail.
      expect(deps.api.downloadFile).toHaveBeenCalledWith('large')
      expect(vision.describe).toHaveBeenCalledWith({ data: new Uint8Array([1, 2, 3]), mimeType: 'image/jpeg' })
      expect(deps.harness.generate).toHaveBeenCalledWith(
        expect.objectContaining({ text: 'thoughts?\n[image: a chart of $TINY, up and to the right]' }),
        [],
      )
    })

    it('answers a photo with no caption at all', async () => {
      const vision: ImageReader = { describe: vi.fn().mockResolvedValue('a cat') }
      const { bot: subject, deps } = build({ vision })

      await subject.handle(photoMessage(dm, { photo }))

      expect(deps.harness.generate).toHaveBeenCalledWith(expect.objectContaining({ text: '[image: a cat]' }), [])
    })

    it('says a photo was sent when nothing can look at it, and keeps going when looking fails', async () => {
      const { bot: blind, deps: blindDeps } = build()
      await blind.handle(photoMessage(dm, { photo }))
      expect(blindDeps.api.downloadFile).not.toHaveBeenCalled()
      expect(blindDeps.harness.generate).toHaveBeenCalledWith(expect.objectContaining({ text: '[sent a photo]' }), [])

      const vision: ImageReader = { describe: vi.fn().mockRejectedValue(new Error('400')) }
      const { bot: subject, deps } = build({ vision })
      await subject.handle(dm('look', { photo }))
      expect(deps.harness.generate).toHaveBeenCalledWith(
        expect.objectContaining({ text: 'look\n[sent a photo that could not be read]' }),
        [],
      )
      expect(deps.api.sendMessage).toHaveBeenCalled()
    })

    it('does not spend a vision call on a group photo that was not for it', async () => {
      const vision: ImageReader = { describe: vi.fn().mockResolvedValue('a cat') }
      const { bot: subject, deps } = build({ vision })

      await subject.handle(photoMessage(group, { caption: 'look at this', photo }))

      expect(vision.describe).not.toHaveBeenCalled()
      expect(deps.harness.generate).not.toHaveBeenCalled()
      // The room still knows a photo went by.
      await subject.handle(group('@teenythebot what was that', {
        message_id: 21,
        entities: [{ type: 'mention', offset: 0, length: 12 }],
      }))
      expect(deps.harness.generate).toHaveBeenCalledWith(expect.anything(), [
        { from: 'person', authorName: 'Alice', text: 'look at this\n[sent a photo]' },
      ])
    })

    it('sends a generated image with the reply as its caption, and remembers what it said', async () => {
      const media = new MediaStore()
      const stored = media.put({ kind: 'image', prompt: 'a tiny robot', data: new Uint8Array([9]), mimeType: 'image/png' })
      const { bot: subject, deps } = build({
        media,
        harness: harness({ text: 'here you go', mediaId: stored.id, incomingTone: 'friendly' }),
      })

      await subject.handle(dm('draw me a tiny robot'))

      expect(deps.api.sendPhoto).toHaveBeenCalledWith(
        1,
        { data: new Uint8Array([9]), mimeType: 'image/png', caption: 'here you go' },
        {},
      )
      expect(deps.api.sendMessage).not.toHaveBeenCalled()
      expect(deps.harness.remember).toHaveBeenCalledWith(expect.anything(), 'here you go', '200', 'friendly')
    })

    it('sends a generated video as a video, and a picture alone when there are no words', async () => {
      const media = new MediaStore()
      const stored = media.put({ kind: 'video', prompt: 'a tiny robot dancing', data: new Uint8Array([9]), mimeType: 'video/mp4' })
      const { bot: subject, deps } = build({
        media,
        harness: harness({ mediaId: stored.id, incomingTone: 'friendly' }),
      })

      await subject.handle(dm('make it dance'))

      expect(deps.api.sendVideo).toHaveBeenCalledWith(1, { data: new Uint8Array([9]), mimeType: 'video/mp4' }, {})
      expect(deps.harness.remember).toHaveBeenCalledWith(expect.anything(), '(sent a generated video)', '201', 'friendly')
    })

    it('falls back to the words when the picture will not send, and when the id is unknown', async () => {
      const media = new MediaStore()
      const stored = media.put({ kind: 'image', prompt: 'x', data: new Uint8Array([9]), mimeType: 'image/png' })
      const failing = api()
      failing.sendPhoto = vi.fn().mockRejectedValue(new Error('413'))
      const { bot: subject, deps } = build({
        media,
        api: failing,
        harness: harness({ text: 'here', mediaId: stored.id, incomingTone: 'friendly' }),
      })
      await subject.handle(dm('draw'))
      expect(deps.api.sendMessage).toHaveBeenCalledWith(1, 'here', {})

      const { bot: other, deps: otherDeps } = build({
        media,
        harness: harness({ text: 'here', mediaId: 'img_notreal_9', incomingTone: 'friendly' }),
      })
      await other.handle(dm('draw'))
      expect(otherDeps.api.sendPhoto).not.toHaveBeenCalled()
      expect(otherDeps.api.sendMessage).toHaveBeenCalledWith(1, 'here', {})
    })
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

describe('TelegramBot content filter', () => {
  it('says nothing at all to a message the filter holds', async () => {
    const { bot: subject, deps } = build({ safety: lexicalGate() })

    await subject.handle(dm('draw yourself naked'))

    expect(deps.harness.generate).not.toHaveBeenCalled()
    expect(deps.api.sendMessage).not.toHaveBeenCalled()
    expect(deps.api.setMessageReaction).not.toHaveBeenCalled()
    expect(deps.harness.remember).not.toHaveBeenCalled()
  })

  it('withholds a draft the filter refuses, reaction included', async () => {
    const { bot: subject, deps } = build({
      safety: lexicalGate(),
      harness: harness({ text: 'this is fucking great', reaction: '🔥', incomingTone: 'friendly' }),
    })

    await subject.handle(dm('hello'))

    expect(deps.api.sendMessage).not.toHaveBeenCalled()
    expect(deps.api.setMessageReaction).not.toHaveBeenCalled()
    expect(deps.harness.remember).not.toHaveBeenCalled()
  })

  it('cuts a link that is not on the allowlist and sends the rest', async () => {
    const { bot: subject, deps } = build({
      safety: lexicalGate(),
      harness: harness({ text: 'claim it at https://claim-tiny.xyz/go fren', incomingTone: 'friendly' }),
    })

    await subject.handle(dm('hello'))

    expect(deps.api.sendMessage).toHaveBeenCalledWith(1, 'claim it at fren', {})
  })

  it('still answers incoming swearing; only its own words are held to that', async () => {
    const { bot: subject, deps } = build({ safety: lexicalGate() })

    await subject.handle(dm('holy shit this is cool'))

    expect(deps.api.sendMessage).toHaveBeenCalledWith(1, 'gm fren', {})
  })

  it('scrubs the history the drafter is shown', async () => {
    const transcript = new MemoryChatTranscript()
    await transcript.append({ chatId: '1', messageId: '1', at: 'a', from: 'person', authorName: 'Bob', text: 'gm all' })
    await transcript.append({ chatId: '1', messageId: '2', at: 'b', from: 'person', authorName: 'Bob', text: 'send nudes' })
    const { bot: subject, deps } = build({ safety: lexicalGate(), transcript })

    await subject.handle(dm('hello'))

    const [, history] = (deps.harness.generate as ReturnType<typeof vi.fn>).mock.calls[0] ?? []
    expect(history.map((turn: { text: string }) => turn.text)).toEqual(['gm all'])
  })
})
