import { describe, expect, it, vi } from 'vitest'
import { ChatHarness } from '../src/harness/chat-harness.js'
import type { ChatMemory, ChatMessage, ChatModel } from '../src/types.js'

const message: ChatMessage = {
  id: '10',
  channel: 'telegram',
  chatId: '1',
  chatKind: 'private',
  text: 'what is $TINY?',
  authorId: '777',
  authorUsername: 'alice',
  authorName: 'Alice',
}

function memory(overrides: Partial<ChatMemory> = {}): ChatMemory {
  return {
    captureChatMessage: vi.fn().mockResolvedValue(undefined),
    recall: vi.fn().mockResolvedValue('alice asked about the roadmap on X last week'),
    captureChatReply: vi.fn().mockResolvedValue(undefined),
    ...overrides,
  }
}

describe('ChatHarness', () => {
  it('recalls the person, drafts, then captures the message', async () => {
    const store = memory()
    const model: ChatModel = { reply: vi.fn().mockResolvedValue({ text: 'a tiny token', incomingTone: 'neutral' }) }
    const harness = new ChatHarness(store, model)

    await expect(harness.generate(message)).resolves.toEqual({ text: 'a tiny token', incomingTone: 'neutral' })
    expect(store.recall).toHaveBeenCalledWith(expect.stringContaining('what is $TINY?'), {
      person: { id: 'user:telegram-777', username: 'alice' },
    })
    expect(model.reply).toHaveBeenCalledWith({
      message,
      memory: expect.stringContaining('alice asked about the roadmap') as unknown as string,
    })
    // Capture happens after the draft, so the recall could not have seen it.
    const recallOrder = (store.recall as ReturnType<typeof vi.fn>).mock.invocationCallOrder[0] ?? 0
    const captureOrder =
      (store.captureChatMessage as ReturnType<typeof vi.fn>).mock.invocationCallOrder[0] ?? 0
    expect(captureOrder).toBeGreaterThan(recallOrder)
  })

  it('hands the drafter the recent conversation and asks memory about it', async () => {
    const store = memory()
    const model: ChatModel = { reply: vi.fn().mockResolvedValue({ text: 'yes' }) }
    const history = [
      { from: 'person' as const, authorName: 'Alice', text: 'do you post charts' },
      { from: 'bot' as const, text: 'every day at 15:00 UTC' },
    ]

    await new ChatHarness(store, model).generate({ ...message, text: 'and where?' }, history)

    expect(model.reply).toHaveBeenCalledWith(expect.objectContaining({ history }))
    expect(store.recall).toHaveBeenCalledWith(
      expect.stringContaining('do you post charts'),
      expect.anything(),
    )
  })

  it('still answers when memory is down', async () => {
    const store = memory({ recall: vi.fn().mockRejectedValue(new Error('401')) })
    const model: ChatModel = { reply: vi.fn().mockResolvedValue({ text: 'hi' }) }

    await expect(new ChatHarness(store, model).generate(message)).resolves.toEqual({ text: 'hi' })
    expect(model.reply).toHaveBeenCalledWith(
      expect.objectContaining({ memory: expect.stringContaining('MEMORY WAS UNAVAILABLE THIS TURN') }),
    )
  })

  it('does not keep a hostile message or the reply to it', async () => {
    const store = memory()
    const model: ChatModel = { reply: vi.fn().mockResolvedValue({ text: 'ok', incomingTone: 'hostile' }) }
    const harness = new ChatHarness(store, model)

    await harness.generate(message)
    await harness.remember(message, 'ok', '11', 'hostile')

    expect(store.captureChatMessage).not.toHaveBeenCalled()
    expect(store.captureChatReply).not.toHaveBeenCalled()
  })

  it('keeps a friendly exchange on both sides', async () => {
    const store = memory()
    const harness = new ChatHarness(store, { reply: vi.fn().mockResolvedValue({ text: 'gm' }) })

    await harness.generate(message)
    await harness.remember(message, 'gm', '11')

    expect(store.captureChatMessage).toHaveBeenCalledWith(message)
    expect(store.captureChatReply).toHaveBeenCalledWith(message, 'gm', '11')
  })
})
