import { describe, expect, it, vi } from 'vitest'
import { buildTools } from '../src/agent/graph.js'
import type { GifSource, Memory, XTools } from '../src/types.js'

/**
 * A prompt rule asks the model not to do something. This asserts it cannot.
 *
 * The account held no keys at all until 2026-09-08, when it was given a wallet
 * on Robinhood Chain to airdrop $TINY. This file used to say that adding one
 * had to be a deliberate decision rather than an inherited ability; this is
 * that decision, written down.
 *
 * What was decided, and what it costs. The airdrop tool IS on the reply path,
 * whose entire input is text written by strangers, because the account is meant
 * to decide in conversation whether somebody has earned a tip. So the old
 * property -- no signer reachable from untrusted text -- is gone, and it is not
 * coming back while that is the product. What replaces it is the third option
 * this file listed: a spend policy enforced outside the model, with a ceiling a
 * turn cannot raise.
 *
 * The boundaries that remain, each asserted below:
 *
 *   - The signer exists only when a wallet is configured. No configuration,
 *     no tool, whatever the prompt says.
 *   - The signer exists only when a turn is bound to a specific post. The
 *     operator endpoint answers a task rather than a post, so it gets no
 *     signer by construction rather than by policy.
 *   - `airdrop_user` is the only capability that spends. There is no approve,
 *     no swap, no arbitrary call, and no way to move the native balance.
 *
 * Everything the model does not control lives in `src/airdrop/policy.ts`, and
 * `test/airdrop.test.ts` is where the limits themselves are asserted. If a
 * second spending tool ever appears here, this test fails, and the same
 * question has to be answered again.
 */

const FORBIDDEN = [
  'wallet',
  'transfer',
  'send_token',
  'sign',
  'approve',
  'swap',
  'bridge',
  'stake',
  'mint',
  'burn',
  'seed',
  'private_key',
  'keypair',
  'transaction',
  'payment',
  'buy',
  'sell',
]

const POST = { id: '1', text: 'gm, my wallet is 0xabc', authorId: '42' }

function options() {
  const memory = {
    captureActivity: vi.fn(),
    captureMention: vi.fn(),
    recall: vi.fn(),
    captureReply: vi.fn(),
  } as unknown as Memory
  const x = {
    searchPosts: vi.fn(),
    getPost: vi.fn(),
    getConversation: vi.fn(),
    createPost: vi.fn(),
    uploadMedia: vi.fn(),
  } as unknown as XTools
  const gifs = { search: vi.fn(), get: vi.fn(), download: vi.fn() } as unknown as GifSource

  return {
    memory,
    x,
    botUsername: 'TinyMascot',
    dryRun: false,
    llm: { baseUrl: 'https://example.com/v1', apiKey: 'k', model: 'm' },
    exaApiKey: 'exa',
    gifs: { source: gifs, channel: 'TinyHumansAI' },
  }
}

/** A wallet whose calls would throw: nothing here should reach them. */
function walletOptions() {
  return {
    ...options(),
    airdrop: {
      wallet: {
        address: '0x0000000000000000000000000000000000000001' as const,
        balance: vi.fn(),
        gasBalance: vi.fn(),
        send: vi.fn(),
      },
      ledger: {
        sentToday: vi.fn(),
        paidRecipientCount: vi.fn(),
        forPost: vi.fn(),
        claim: vi.fn(),
        settle: vi.fn(),
        fail: vi.fn(),
        close: vi.fn(),
      },
      limits: {
        maxPerAirdrop: 100,
        maxPerDay: 1000,
        maxPerRecipient: 1,
        requireAddressInPost: true,
      },
      symbol: 'TINY',
    },
  } as unknown as Parameters<typeof buildTools>[0]
}

describe('agent capabilities', () => {
  it('exposes no tool that could touch a wallet or move funds', () => {
    for (const tool of buildTools(options())) {
      const surface = `${tool.name} ${tool.description}`.toLowerCase()
      for (const word of FORBIDDEN) {
        expect(
          tool.name.toLowerCase().includes(word),
          `tool "${tool.name}" looks like a wallet capability`,
        ).toBe(false)
      }
      expect(surface).not.toContain('seed phrase')
      expect(surface).not.toContain('private key')
    }
  })

  it('is the exact published tool list, so a new capability is a conscious change', () => {
    expect(buildTools(options()).map((tool) => tool.name)).toEqual([
      'recall_memory',
      'remember',
      'x_search_posts',
      'x_get_post',
      'x_get_comments',
      'x_post',
      'web_search',
      'read_web_pages',
      'find_similar_pages',
      'web_answer',
      'find_gif',
    ])
  })

  it('has no signer when no wallet is configured, whatever the prompt says', () => {
    expect(buildTools(options(), () => ({ post: POST })).map((tool) => tool.name)).not.toContain(
      'airdrop_user',
    )
  })

  // The operator endpoint answers a task, not a post, so there is nothing to
  // bind a payment to and nothing to check an address against. It gets no
  // signer by construction rather than by remembering to configure one.
  it('has no signer on a turn that is not bound to a post', () => {
    expect(buildTools(walletOptions()).map((tool) => tool.name)).not.toContain('airdrop_user')
  })

  // Two tools, one of which spends: airdrop_check is a read that exists so the
  // account can answer "can I have some?" without promising what the policy
  // would then refuse.
  it('adds exactly one spending tool when a wallet is configured and a post is bound', () => {
    const withWallet = buildTools(walletOptions(), () => ({ post: POST })).map((tool) => tool.name)
    const without = buildTools(options()).map((tool) => tool.name)

    expect(withWallet.filter((name) => !without.includes(name))).toEqual([
      'airdrop_check',
      'airdrop_user',
    ])
  })
})
