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

    await expect(harness.generate(post)).resolves.toMatchObject({ text: 'An open-source dapp.' })
    expect(memory.captureMention).toHaveBeenCalledWith(post)
    expect(memory.recall).toHaveBeenCalledWith(expect.any(String), {
      person: { id: 'user:x-777', username: 'web3knight' },
    })
    expect(model.reply).toHaveBeenCalledWith({
      post,
      memory: expect.stringContaining('stonks.rocks is an open-source dapp') as unknown as string,
    })
  })

  /**
   * Asked "much higher for tiny?", the account replied "I'm just a mascot who
   * doesn't read charts" -- hours after publishing a chart of its own token.
   * Nothing had ever told it what it had posted.
   */
  it('also recalls what the account itself has posted, and labels it as such', async () => {
    const memory: Memory = {
      captureActivity: vi.fn().mockResolvedValue(undefined),
      captureMention: vi.fn().mockResolvedValue(undefined),
      recall: vi
        .fn()
        .mockResolvedValueOnce('they asked about the roadmap last week')
        .mockResolvedValueOnce('yesterday you posted the daily chart'),
      captureReply: vi.fn().mockResolvedValue(undefined),
    }
    const model: LanguageModel = { reply: vi.fn().mockResolvedValue({ text: 'hi' }) }

    await new ReplyHarness(memory, model).generate({ id: '1', text: 'much higher for tiny?' })

    const passed = (model.reply as ReturnType<typeof vi.fn>).mock.calls[0]?.[0] as { memory: string }
    expect(passed.memory).toContain('they asked about the roadmap last week')
    expect(passed.memory).toContain('WHAT YOU YOURSELF HAVE POSTED RECENTLY')
    expect(passed.memory).toContain('yesterday you posted the daily chart')
  })

  it('drops the section entirely rather than showing an empty heading', async () => {
    const memory: Memory = {
      captureActivity: vi.fn().mockResolvedValue(undefined),
      captureMention: vi.fn().mockResolvedValue(undefined),
      recall: vi.fn().mockResolvedValueOnce('context').mockResolvedValueOnce('   '),
      captureReply: vi.fn().mockResolvedValue(undefined),
    }
    const model: LanguageModel = { reply: vi.fn().mockResolvedValue({ text: 'hi' }) }

    await new ReplyHarness(memory, model).generate({ id: '1', text: 'hello' })
    const passed = (model.reply as ReturnType<typeof vi.fn>).mock.calls[0]?.[0] as { memory: string }
    expect(passed.memory).toBe('context')
  })

  /** A memory outage on the second recall must not cost the reply the first. */
  it('still replies when the self-recall fails', async () => {
    const memory: Memory = {
      captureActivity: vi.fn().mockResolvedValue(undefined),
      captureMention: vi.fn().mockResolvedValue(undefined),
      recall: vi
        .fn()
        .mockResolvedValueOnce('context')
        .mockRejectedValueOnce(new Error('401 MISSING_TOKEN')),
      captureReply: vi.fn().mockResolvedValue(undefined),
    }
    const model: LanguageModel = { reply: vi.fn().mockResolvedValue({ text: 'hi' }) }

    await expect(
      new ReplyHarness(memory, model).generate({ id: '1', text: 'hello' }),
    ).resolves.toMatchObject({ text: 'hi' })
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
    // Two recalls now -- the post's context and the account's own recent posts
    // -- but the invariant is unchanged: every recall happens before the
    // capture, or the agent reads the post it is answering back as history.
    expect(order.at(-1)).toBe('capture')
    expect(order.filter((step) => step === 'recall')).toHaveLength(2)
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

/**
 * CortexDB returning 401 used to take the whole reply down. The mention is then
 * marked failed by the queue and nothing retries it, so a memory outage does
 * not degrade the reply -- it silently drops the person from the conversation.
 */
describe('when memory is down', () => {
  const broken = (): Memory => ({
    captureActivity: vi.fn().mockRejectedValue(new Error('401 MISSING_TOKEN')),
    captureMention: vi.fn().mockRejectedValue(new Error('401 MISSING_TOKEN')),
    recall: vi.fn().mockRejectedValue(new Error('401 MISSING_TOKEN')),
    captureReply: vi.fn().mockRejectedValue(new Error('401 MISSING_TOKEN')),
  })

  it('still drafts a reply', async () => {
    const model: LanguageModel = { reply: vi.fn().mockResolvedValue({ text: 'hi' }) }
    await expect(
      new ReplyHarness(broken(), model).generate({ id: '1', text: 'gm' }),
    ).resolves.toMatchObject({ text: 'hi' })
  })

  it('tells the drafter it has no memory rather than passing an error through', async () => {
    const model: LanguageModel = { reply: vi.fn().mockResolvedValue({ text: 'hi' }) }
    await new ReplyHarness(broken(), model).generate({ id: '1', text: 'gm' })

    const passed = (model.reply as ReturnType<typeof vi.fn>).mock.calls[0]?.[0] as { memory: string }
    expect(passed.memory).toBe('')
  })

  it('does not fail the reply because the mention could not be stored', async () => {
    const model: LanguageModel = { reply: vi.fn().mockResolvedValue({ text: 'hi' }) }
    const memory = broken()
    await expect(
      new ReplyHarness(memory, model).generate({ id: '1', text: 'gm', authorId: '9' }),
    ).resolves.toBeDefined()
    expect(memory.captureMention).toHaveBeenCalled()
  })
})
