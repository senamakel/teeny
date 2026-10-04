import { describe, expect, it, vi } from 'vitest'
import { InMemoryPostLedger } from '../src/cadence/ledger.js'
import { OWN_ACTIVITY_HEADING, OWN_ACTIVITY_QUERY, TurnContext } from '../src/memory/context.js'
import type { Exchange } from '../src/queue/mentions.js'

const now = Date.parse('2026-10-05T12:00:00Z')

function ledgerWith(posts: Array<{ postId: string; kind: string; postedAt: string; text?: string }>) {
  const ledger = new InMemoryPostLedger()
  return Promise.all(posts.map((post) => ledger.record(post))).then(() => ledger)
}

describe('TurnContext', () => {
  it('reads the exchanges and the own posts exactly, and asks memory only for the rest', async () => {
    const recall = vi.fn().mockResolvedValue('ABOUT @ana:\nlikes the lab lore')
    const exchanges = vi.fn(async (): Promise<Exchange[]> => [
      { at: '2026-10-04T09:00:00Z', incoming: 'is the serial back?', reply: 'tuesday, promise' },
    ])
    const ledger = await ledgerWith([
      { postId: 'p1', kind: 'chart', postedAt: '2026-10-05T07:00:00Z', text: 'the chart today' },
      { postId: 'p0', kind: 'morning', postedAt: '2026-09-20T07:00:00Z', text: 'too old to show' },
    ])
    const context = new TurnContext({ memory: { recall }, ownPosts: ledger, exchanges, now: () => now })

    const turn = await context.gather({ query: 'q', person: { id: 'user:x-7', username: 'ana' }, authorId: '7', postId: '99' })

    expect(exchanges).toHaveBeenCalledWith('7', 6, { excludePostId: '99' })
    // One recall, not two: the own-activity question is answered by the ledger.
    expect(recall).toHaveBeenCalledTimes(1)
    expect(turn.text).toContain('YOUR LAST 1 EXCHANGES WITH @ana')
    expect(turn.text).toContain('[2026-10-04 09:00 UTC] @ana: is the serial back?\n  you: tuesday, promise')
    expect(turn.text).toContain('likes the lab lore')
    expect(turn.text).toContain(`${OWN_ACTIVITY_HEADING}\n[2026-10-05 07:00 UTC · chart] the chart today`)
    expect(turn.text).not.toContain('too old to show')
    expect(turn.degraded).toEqual([])
  })

  it('says memory was unavailable instead of passing silence as "nothing known"', async () => {
    const warn = vi.fn()
    const context = new TurnContext({
      memory: { recall: vi.fn().mockRejectedValue(new Error('request timed out after 15000ms')) },
      ownPosts: await ledgerWith([{ postId: 'p1', kind: 'moon', postedAt: '2026-10-05T03:00:00Z', text: 'moon chart' }]),
      now: () => now,
      logger: { warn },
    })

    const turn = await context.gather({ query: 'q', person: { id: 'user:x-7' } })

    expect(turn.text).toContain('MEMORY WAS UNAVAILABLE THIS TURN')
    // The other sources still arrive.
    expect(turn.text).toContain('moon chart')
    expect(turn.degraded).toEqual(['memory'])
    expect(turn.recalled).toContain('(memory unavailable)')
    expect(warn).toHaveBeenCalledOnce()
  })

  it('still drafts from memory when the exchange history cannot be read', async () => {
    const context = new TurnContext({
      memory: { recall: vi.fn().mockResolvedValue('GENERAL CONTEXT:\nfacts') },
      exchanges: vi.fn().mockRejectedValue(new Error('mongo down')),
      ownPosts: await ledgerWith([]),
      now: () => now,
    })

    const turn = await context.gather({ query: 'q', authorId: '7' })

    expect(turn.text).toBe('GENERAL CONTEXT:\nfacts')
    expect(turn.degraded).toEqual(['exchanges'])
  })

  it('falls back to asking memory for its own posts when there is no ledger', async () => {
    const recall = vi.fn(async (query: string) => (query === OWN_ACTIVITY_QUERY ? 'posted a chart' : 'context'))
    const turn = await new TurnContext({ memory: { recall } }).gather({ query: 'q' })

    expect(recall).toHaveBeenCalledWith(OWN_ACTIVITY_QUERY)
    expect(turn.text).toBe(`context\n\n${OWN_ACTIVITY_HEADING}\nposted a chart`)
  })

  it('links a person to their ids on other channels, both ways', () => {
    const context = new TurnContext({
      memory: { recall: vi.fn() },
      links: [['user:x-7', 'user:telegram-9']],
    })

    expect(context.linked({ id: 'user:x-7' }).aliases).toEqual(['user:telegram-9'])
    expect(context.linked({ id: 'user:telegram-9' }).aliases).toEqual(['user:x-7'])
    expect(context.linked({ id: 'user:x-8' }).aliases).toBeUndefined()
  })

  it('passes the linked person to memory', async () => {
    const recall = vi.fn().mockResolvedValue('')
    const context = new TurnContext({ memory: { recall }, links: [['user:x-7', 'user:telegram-9']], ownPosts: await ledgerWith([]) })

    await context.gather({ query: 'q', person: { id: 'user:telegram-9', username: 'ana' } })

    expect(recall).toHaveBeenCalledWith('q', { person: { id: 'user:telegram-9', username: 'ana', aliases: ['user:x-7'] } })
  })

  it('turns a source off at zero', async () => {
    const exchanges = vi.fn()
    const recall = vi.fn().mockResolvedValue('')
    const context = new TurnContext({
      memory: { recall },
      exchanges,
      limits: { exchanges: 0, ownPostsLimit: 0 },
    })

    await context.gather({ query: 'q', authorId: '7' })

    expect(exchanges).not.toHaveBeenCalled()
    expect(recall).toHaveBeenCalledTimes(1)
  })
})
