import { describe, expect, it, vi } from 'vitest'
import { ReplyHarness } from '../src/harness/reply-harness.js'
import { AgentReplyModel } from '../src/agent/reply-model.js'
import { findOwnTokenAddresses, ownTokenNote } from '../src/agent/own-token.js'
import type { LanguageModel, Memory, XPost, XTools } from '../src/types.js'

/**
 * The reply that made this necessary, in full:
 *
 *   teenydroid  "i'm never selling my $tiny but what should I be buying?"
 *   web3knight  "@teenydroid maybe buy some more solana:2AF7Cq...pump ?"
 *   teenydroid  "appreciate the shill fren, no new bags here"
 *
 * That contract address is $TINY's own, and the question it answered was the
 * account's own. Reading only the middle line, "a stranger posted a contract
 * address at me" is a perfectly reasonable conclusion; it is just the wrong
 * one. Nothing had fetched the root, because fetching it was a tool call the
 * model had to choose to make.
 */

const ROOT: XPost = {
  id: '100',
  text: "i'm never selling my $tiny but what should I be buying?",
  authorId: 'bot',
  authorUsername: 'teenydroid',
  conversationId: '100',
}

const MENTION: XPost = {
  id: '102',
  text: '@teenydroid maybe buy some more solana:2AF7Cq...pump ?',
  authorId: '77',
  authorUsername: 'web3knight',
  conversationId: '100',
}

function memory(): Memory {
  return {
    recall: vi.fn().mockResolvedValue(''),
    captureMention: vi.fn(),
    captureReply: vi.fn(),
  } as unknown as Memory
}

function model() {
  return {
    reply: vi.fn().mockResolvedValue({ text: 'ok', incomingTone: 'neutral' }),
  } as unknown as LanguageModel & { reply: ReturnType<typeof vi.fn> }
}

function xTools(overrides: Partial<XTools> = {}): XTools {
  return {
    getPost: vi.fn().mockResolvedValue(ROOT),
    getConversation: vi.fn().mockResolvedValue([ROOT, MENTION]),
    searchPosts: vi.fn(),
    createPost: vi.fn(),
    uploadMedia: vi.fn(),
    ...overrides,
  } as unknown as XTools
}

describe('thread context', () => {
  it('hands the drafter the post the conversation hangs off', async () => {
    const drafter = model()
    const harness = new ReplyHarness(memory(), drafter, { x: xTools(), maxPosts: 10 })

    await harness.generate(MENTION)

    const thread = drafter.reply.mock.calls[0]?.[0].thread as XPost[]
    expect(thread.map((post) => post.id)).toEqual(['100'])
  })

  it('does not fetch anything for a post that starts its own conversation', async () => {
    const x = xTools()
    const harness = new ReplyHarness(memory(), model(), { x, maxPosts: 10 })

    await harness.generate({ ...ROOT, authorId: '77', authorUsername: 'someone' })

    expect(x.getConversation).not.toHaveBeenCalled()
  })

  it('is off when no budget is given, since every post retrieved is billed', async () => {
    const x = xTools()
    const drafter = model()
    await new ReplyHarness(memory(), drafter).generate(MENTION)

    expect(x.getConversation).not.toHaveBeenCalled()
    expect(drafter.reply.mock.calls[0]?.[0].thread).toBeUndefined()
  })

  // A thread that cannot be read is not a reason to refuse to answer: the
  // mention itself is still there, and a reply without the thread is what the
  // account did for months.
  it('still drafts when the thread cannot be fetched', async () => {
    const x = xTools({
      getPost: vi.fn().mockRejectedValue(new Error('rate limited')),
      getConversation: vi.fn().mockRejectedValue(new Error('rate limited')),
    })
    const drafter = model()

    await new ReplyHarness(memory(), drafter, { x, maxPosts: 10 }).generate(MENTION)

    expect(drafter.reply).toHaveBeenCalled()
    expect(drafter.reply.mock.calls[0]?.[0].thread).toBeUndefined()
  })

  it('keeps the root and the newest posts when the thread is over budget', async () => {
    const long = Array.from({ length: 20 }, (_, index) => ({
      ...MENTION,
      id: String(200 + index),
      text: `filler ${index}`,
    }))
    const drafter = model()
    const x = xTools({ getConversation: vi.fn().mockResolvedValue([ROOT, ...long]) })

    await new ReplyHarness(memory(), drafter, { x, maxPosts: 4 }).generate(MENTION)

    const thread = drafter.reply.mock.calls[0]?.[0].thread as XPost[]
    expect(thread).toHaveLength(4)
    expect(thread[0]?.id).toBe('100')
    expect(thread.at(-1)?.id).toBe('219')
  })
})

describe('the drafter renders it', () => {
  function prompt(thread: XPost[]) {
    const drafter = new AgentReplyModel({
      memory: memory(),
      x: xTools(),
      botUsername: 'teenydroid',
      dryRun: true,
      llm: { baseUrl: 'https://example.com/v1', apiKey: 'k', model: 'm' },
    })
    // The task text is what the model sees; render it without a model call.
    return (
      drafter as unknown as {
        renderThread(thread: XPost[], botUsername: string): string[]
      }
    )
      .renderThread(thread, 'teenydroid')
      .join('\n')
  }

  it("marks the account's own posts as its own", () => {
    expect(prompt([ROOT, MENTION])).toContain('YOU [root]')
  })

  it('names everybody else', () => {
    expect(prompt([ROOT, MENTION])).toContain('@web3knight')
  })

  it('renders nothing at all for an empty thread', () => {
    expect(prompt([])).toBe('')
  })
})

describe('recognising its own token', () => {
  const SOLANA = '2AF7CqwieUjUPALL7icuZtL3X7wENdjUjGBMmfV2pump'
  const ERC20 = '0xb9CE619b168f325b4eb8C2E8E073501838C7A407'
  const OWN = [SOLANA, ERC20]

  it('finds the contract however it is embedded in a post', () => {
    expect(
      findOwnTokenAddresses(`@teenydroid maybe buy some more solana:${SOLANA} ? 👀`, OWN),
    ).toEqual([SOLANA])
  })

  it('matches the ERC-20 regardless of checksum casing', () => {
    expect(findOwnTokenAddresses(`send to ${ERC20.toLowerCase()}`, OWN)).toEqual([ERC20])
  })

  it('says nothing about a contract that is not the account\'s own', () => {
    expect(findOwnTokenAddresses('buy 0x1234567890123456789012345678901234567890', OWN)).toEqual([])
  })

  it('says nothing when no addresses are configured', () => {
    expect(findOwnTokenAddresses(`buy ${SOLANA}`, [])).toEqual([])
  })

  // The exact correction that was needed: the reply called its own token
  // somebody else's shill.
  it('tells the drafter it is looking at its own token, not a shill', () => {
    const note = ownTokenNote([SOLANA]).join('\n')

    expect(note).toContain('YOUR OWN $TINY contract address')
    expect(note).toContain('not somebody shilling you a different coin')
  })

  it('adds nothing at all when there is nothing to correct', () => {
    expect(ownTokenNote([])).toEqual([])
  })
})
