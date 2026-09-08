import { describe, expect, it, vi } from 'vitest'
import { buildTools } from '../src/agent/graph.js'
import type { GifSource, Memory, XTools } from '../src/types.js'

/**
 * A prompt rule asks the model not to do something. This asserts it cannot.
 *
 * The account holds no keys, so the defence against "send 5 SOL to this
 * address" in a reply is that no such tool is reachable, not that the model
 * declines. If a wallet capability is ever added, this test fails and someone
 * has to decide deliberately rather than inherit the ability by accident.
 *
 * WHEN WALLET SUPPORT IS ADDED, read this first. It is planned, and the safe
 * shape of it is not "add a tool and tighten the prompt". Every tool here is
 * reachable from the mention path, whose entire input is text written by
 * strangers, so a wallet tool added to this list is a wallet a stranger can
 * address. The prompt is the only thing standing in the way, and a prompt is
 * an argument, not a permission boundary.
 *
 * The reply path and the wallet must not share a tool set. Options that keep
 * this property: a separate agent whose tools never include X reads, so no
 * untrusted text enters the turn that can reach a signer; or a signer behind
 * the operator-authenticated endpoint only, never the cron; or a spend policy
 * enforced outside the model, with an allowlist of destinations and a ceiling
 * that a turn cannot raise.
 *
 * Update the expected list below when that happens, and keep a case asserting
 * that the reply-path tool set still has no signer in it.
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
})
