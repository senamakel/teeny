import { describe, expect, it, vi } from 'vitest'
import { buildTools } from '../src/agent/graph.js'
import { createMarketTools } from '../src/agent/tools/market.js'
import type { ChartRenderer } from '../src/chart/renderer.js'
import type { TokenMarket } from '../src/market/market.js'
import { MediaBudget, MediaStore } from '../src/media/store.js'
import type { GifSource, Memory, XTools } from '../src/types.js'

/**
 * A prompt rule asks the model not to do something. This asserts it cannot.
 *
 * The account holds no keys, so the defence against "send 5 SOL to this
 * address" in a reply is that no such tool is reachable, not that the model
 * declines. If a wallet capability is ever added, this test fails and someone
 * has to decide deliberately rather than inherit the ability by accident.
 *
 * It was added, on 2026-09-08, and removed again the same day. Both halves are
 * worth recording, because the reasoning above turned out to be right for a
 * reason nobody had written down.
 *
 * The wallet worked exactly as designed: the caps held, the address-in-post
 * rule held, one wallet could not farm many accounts, and 106 airdrops went out
 * with nothing lost or overpaid. What went wrong was not the spending. It was
 * that a hundred near-identical reply-and-pay posts in an hour is the shape of
 * spam whatever the intent, and X shadowbanned the account for it. The risk on
 * a giveaway is not only that somebody talks the model into overpaying -- it is
 * that doing it at all, at speed, in public, costs the account its reach.
 *
 * So the tool is gone and the machinery is not: `src/airdrop/` and
 * `src/clients/wallet.ts` still hold the policy, the ledger, the recipient
 * registry and the ERC-20 client, all still tested. Nothing constructs them,
 * and no turn can reach a signer. Bringing it back means wiring a tool in
 * again, which is a deliberate act that trips this test -- which is the whole
 * point of it.
 *
 * If that day comes, read the git history first: the safe shape was a spend
 * policy enforced outside the model, and the thing that actually bit was rate
 * and repetition, not the ceiling.
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

  /**
   * Seeing, drawing and filming were added on 2026-09-14. Each tool appears
   * only with its provider, so a box with none of the keys still publishes
   * the list above. None of them spends anything but the provider's bill,
   * and that bill has its own daily cap in `MediaBudget`.
   */
  it('adds exactly the media tools that are configured, and nothing else', () => {
    const base = options()
    const store = new MediaStore()
    const budget = new MediaBudget({ imagesPerDay: 1, videosPerDay: 1 })
    const reader = { describe: vi.fn() }
    const images = { generate: vi.fn() }
    const videos = { generate: vi.fn() }

    const names = (media: Parameters<typeof buildTools>[0]['media']) =>
      buildTools({ ...base, ...(media ? { media } : {}) })
        .map((tool) => tool.name)
        .filter((name) => !['recall_memory', 'remember', 'x_search_posts', 'x_get_post', 'x_get_comments', 'x_post', 'web_search', 'read_web_pages', 'find_similar_pages', 'web_answer', 'find_gif'].includes(name))

    expect(names({ store, budget })).toEqual([])
    expect(names({ store, budget, reader })).toEqual(['look_at_image'])
    expect(names({ store, budget, images })).toEqual(['generate_image'])
    expect(names({ store, budget, reader, images, videos })).toEqual([
      'look_at_image',
      'generate_image',
      'generate_video',
    ])
    for (const tool of buildTools({ ...base, media: { store, budget, reader, images, videos } })) {
      for (const word of FORBIDDEN) {
        expect(tool.name.toLowerCase().includes(word), `tool "${tool.name}"`).toBe(false)
      }
    }
  })

  /**
   * The chart agent came later and has its own tool set: market data, on-chain
   * flow, a renderer and one publish tool. It reads posts and trade labels
   * written by strangers, so it is exactly as reachable by a prompt injection
   * as the reply drafter is, and it gets the same guarantee.
   */
  it('gives the chart agent no way to spend either', () => {
    const tools = createMarketTools({
      market: {} as TokenMarket,
      address: 'TOKEN',
      x: options().x,
      charts: { name: 'test', render: vi.fn() } as unknown as ChartRenderer,
      dryRun: true,
    })

    for (const tool of tools) {
      for (const word of FORBIDDEN) {
        expect(
          tool.name.toLowerCase().includes(word),
          `tool "${tool.name}" looks like a wallet capability`,
        ).toBe(false)
      }
    }
  })

  // The list above is the whole of it, and none of these words appear in it.
  it('has no tool that spends, however the turn is built', () => {
    const names = buildTools(options()).map((tool) => tool.name)

    for (const name of names) {
      expect(name).not.toMatch(/airdrop|wallet|send|pay|transfer/i)
    }
  })
})

/**
 * Who may generate, decided per turn. The Telegram drafter never; the X
 * drafter only when the person asked; a calendar item only for its own kind.
 * Seeing is never taken away.
 */
describe('media gating', () => {
  const store = new MediaStore()
  const budget = new MediaBudget({ imagesPerDay: 1, videosPerDay: 1 })
  const full = { store, budget, reader: { describe: vi.fn() }, images: { generate: vi.fn() }, videos: { generate: vi.fn() } }
  const generators = (media: Parameters<typeof buildTools>[0]['media']) =>
    buildTools({ ...options(), ...(media ? { media } : {}) })
      .map((tool) => tool.name)
      .filter((name) => name.startsWith('generate_') || name === 'look_at_image')

  it('takes the generators away and leaves the eyes', async () => {
    const { gateMedia } = await import('../src/media/wiring.js')
    expect(generators(gateMedia(full, {}))).toEqual(['look_at_image'])
    expect(generators(gateMedia(full, { images: true }))).toEqual(['look_at_image', 'generate_image'])
    expect(generators(gateMedia(full, { videos: true }))).toEqual(['look_at_image', 'generate_video'])
    expect(generators(gateMedia(full, { images: true, videos: true }))).toEqual([
      'look_at_image',
      'generate_image',
      'generate_video',
    ])
    expect(gateMedia(undefined, { images: true })).toBeUndefined()
    // The same store and budget on both sides of the gate.
    expect(gateMedia(full, {})?.store).toBe(store)
  })
})

describe('the explicit-ask gate', () => {
  it('asks the judge, and answers no when the judge cannot be reached', async () => {
    const { MediaGate } = await import('../src/agent/media-gate.js')
    const llm = { baseUrl: 'https://example.com/v1', apiKey: 'k', model: 'm' }
    const judge = { invoke: vi.fn().mockResolvedValue({ image: true, video: false }) }
    await expect(new MediaGate(llm, judge).asks('@teeny draw me a tiny on the moon')).resolves.toEqual({
      image: true,
      video: false,
    })
    expect(judge.invoke).toHaveBeenCalledWith([
      expect.objectContaining({ role: 'system' }),
      { role: 'user', content: 'POST:\n@teeny draw me a tiny on the moon' },
    ])

    const down = { invoke: vi.fn().mockRejectedValue(new Error('503')) }
    await expect(new MediaGate(llm, down).asks('anything')).resolves.toEqual({ image: false, video: false })
    await expect(new MediaGate(llm, judge).asks('   ')).resolves.toEqual({ image: false, video: false })
  })
})
