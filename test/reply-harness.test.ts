import { describe, expect, it, vi } from 'vitest'
import { ReplyHarness } from '../src/harness/reply-harness.js'
import type { LanguageModel, Memory, XPost } from '../src/types.js'

describe('ReplyHarness', () => {
  it('captures the mention, recalls context, and generates a grounded reply', async () => {
    const post: XPost = {
      id: '42',
      text: '@stonksrocks what is this?',
      authorId: '777',
      authorUsername: 'web3knight',
    }
    const memory: Memory = {
      captureActivity: vi.fn().mockResolvedValue(undefined),
    captureMention: vi.fn().mockResolvedValue(undefined),
      recall: vi.fn().mockResolvedValue('stonks.rocks is an open-source dapp'),
      captureReply: vi.fn().mockResolvedValue(undefined),
    }
    const model: LanguageModel = {
      reply: vi.fn().mockResolvedValue({ text: 'An open-source dapp.' }),
    }
    const harness = new ReplyHarness(memory, model)

    await expect(harness.generate(post)).resolves.toEqual({ text: 'An open-source dapp.' })
    expect(memory.captureMention).toHaveBeenCalledWith(post)
    expect(memory.recall).toHaveBeenCalledWith(expect.any(String), {
      person: { id: 'user:x-777', username: 'web3knight' },
    })
    expect(model.reply).toHaveBeenCalledWith({
      post,
      memory: 'stonks.rocks is an open-source dapp',
    })
  })
})

describe('ReplyHarness without an author id', () => {
  it('still recalls, just without a person filter', async () => {
    const post: XPost = { id: '43', text: 'hello' }
    const memory: Memory = {
      captureActivity: vi.fn().mockResolvedValue(undefined),
      captureMention: vi.fn().mockResolvedValue(undefined),
      recall: vi.fn().mockResolvedValue('context'),
      captureReply: vi.fn().mockResolvedValue(undefined),
    }
    const harness = new ReplyHarness(memory, {
      reply: vi.fn().mockResolvedValue({ text: 'hi' }),
    })

    await harness.generate(post)
    expect(memory.recall).toHaveBeenCalledWith(expect.any(String), {})
  })
})

describe('recall ordering', () => {
  it('recalls before capturing, so the post is not read back as history', async () => {
    const order: string[] = []
    const memory: Memory = {
      captureActivity: vi.fn(),
      captureMention: vi.fn().mockImplementation(async () => {
        order.push('capture')
      }),
      recall: vi.fn().mockImplementation(async () => {
        order.push('recall')
        return 'context'
      }),
      captureReply: vi.fn(),
    }
    const harness = new ReplyHarness(memory, { reply: vi.fn().mockResolvedValue({ text: 'hi' }) })

    await harness.generate({ id: '1', text: 'gm', authorId: '9', authorUsername: 'someone' })
    expect(order).toEqual(['recall', 'capture'])
  })
})

describe('negative posts', () => {
  function memoryFake(): Memory {
    return {
      captureActivity: vi.fn(),
      captureMention: vi.fn().mockResolvedValue(undefined),
      recall: vi.fn().mockResolvedValue('context'),
      captureReply: vi.fn().mockResolvedValue(undefined),
    }
  }

  it.each(['negative', 'hostile'] as const)(
    'answers a %s post without remembering it',
    async (incomingTone) => {
      const memory = memoryFake()
      const harness = new ReplyHarness(memory, {
        reply: vi.fn().mockResolvedValue({ text: 'noted', incomingTone }),
      })
      const post: XPost = { id: '1', text: 'this project is a scam', authorId: '9' }

      const draft = await harness.generate(post)
      expect(draft.text).toBe('noted')
      expect(memory.captureMention).not.toHaveBeenCalled()

      await harness.remember(post, 'noted', '77', draft.incomingTone)
      expect(memory.captureReply).not.toHaveBeenCalled()
    },
  )

  it('remembers friendly and neutral posts as usual', async () => {
    for (const incomingTone of ['friendly', 'neutral', undefined] as const) {
      const memory = memoryFake()
      const harness = new ReplyHarness(memory, {
        reply: vi.fn().mockResolvedValue({ text: 'gm', ...(incomingTone ? { incomingTone } : {}) }),
      })
      const post: XPost = { id: '1', text: 'gm', authorId: '9' }

      await harness.generate(post)
      expect(memory.captureMention).toHaveBeenCalledWith(post)

      await harness.remember(post, 'gm', '77', incomingTone)
      expect(memory.captureReply).toHaveBeenCalled()
    }
  })
})
