import { describe, expect, it, vi } from 'vitest'
import { SafetyGate, lexicalGate } from '../src/safety/gate.js'
import { MemoryHoldLog, type HoldInput } from '../src/safety/holds.js'
import { allowlistEntryFor, reviewHold } from '../src/safety/review.js'
import { buildServer } from '../src/server.js'
import type { Memory } from '../src/types.js'

function hold(overrides: Partial<HoldInput> = {}): HoldInput {
  return {
    direction: 'inbound',
    kind: 'message',
    context: { surface: 'x', postId: '10', authorUsername: 'alice' },
    category: 'sexual',
    reason: 'contains "naked"',
    by: 'screen',
    text: 'draw yourself naked',
    ...overrides,
  }
}

describe('the gate records what it holds', () => {
  it('writes an inbound hold with the text and where it came from', async () => {
    const holds = new MemoryHoldLog()
    const gate = lexicalGate({ holds })
    await gate.inbound('send nudes', { surface: 'x', postId: '10', authorUsername: 'alice' })

    const [entry] = await holds.list({})
    expect(entry).toMatchObject({
      direction: 'inbound',
      kind: 'message',
      category: 'sexual',
      by: 'screen',
      text: 'send nudes',
      status: 'pending',
      context: { surface: 'x', postId: '10', authorUsername: 'alice' },
    })
  })

  it('writes a task hold for the ad-hoc endpoint', async () => {
    const holds = new MemoryHoldLog()
    await lexicalGate({ holds }).inbound('send nudes', { surface: 'task' })
    expect((await holds.list({}))[0]?.kind).toBe('task')
  })

  it('writes a link hold when a draft goes out with links cut', async () => {
    const holds = new MemoryHoldLog()
    const result = await lexicalGate({ holds }).outbound('see https://claim-tiny.xyz/go now', { surface: 'x', postId: '10' })
    expect(result.decision.ok).toBe(true)

    const [entry] = await holds.list({})
    expect(entry).toMatchObject({ kind: 'link', category: 'links', by: 'links', text: 'see https://claim-tiny.xyz/go now' })
    expect(entry?.links).toEqual([{ link: 'https://claim-tiny.xyz/go', reason: 'claim-tiny.xyz is not on the link allowlist' }])
  })

  it('writes a draft hold for a withheld reply, and a prompt hold for a refused picture', async () => {
    const holds = new MemoryHoldLog()
    const gate = lexicalGate({ holds })
    await gate.outbound('this is fucking great', { surface: 'telegram', chatId: '1', messageId: '5' })
    await gate.outbound('Tiny naked on a beach', { surface: 'x', kind: 'prompt', tool: 'generate_image' })

    const entries = await holds.list({})
    expect(entries.map((entry) => entry.kind)).toEqual(['prompt', 'draft'])
  })

  it('records nothing without a context, and nothing when the text is clean', async () => {
    const holds = new MemoryHoldLog()
    const gate = lexicalGate({ holds })
    await gate.inbound('send nudes')
    await gate.inbound('gm', { surface: 'x' })
    await gate.outbound('gm', { surface: 'x' })
    expect(await holds.list({})).toEqual([])
  })

  it('never lets a broken log turn a hold into anything else', async () => {
    const holds = new MemoryHoldLog()
    vi.spyOn(holds, 'record').mockRejectedValue(new Error('mongo down'))
    const decision = await lexicalGate({ holds }).inbound('send nudes', { surface: 'x' })
    expect(decision.ok).toBe(false)
  })
})

describe('approved links reach the gate', () => {
  it('reads operator approvals alongside the built-in list', async () => {
    const holds = new MemoryHoldLog()
    await holds.allowLink('claim-tiny.xyz', { holdId: 'h1' })
    const result = await lexicalGate({ holds }).outbound('see https://claim-tiny.xyz/go', { surface: 'x' })
    expect(result.text).toBe('see https://claim-tiny.xyz/go')
    expect(result.fixes).toEqual([])
  })

  it('caches them, and refreshes on demand', async () => {
    const holds = new MemoryHoldLog()
    const gate = lexicalGate({ holds })
    expect((await gate.outbound('https://claim-tiny.xyz/go ok', { surface: 'x' })).fixes).toHaveLength(1)
    await holds.allowLink('claim-tiny.xyz', { holdId: 'h1' })
    // Still cached.
    expect((await gate.outbound('https://claim-tiny.xyz/go ok')).fixes).toHaveLength(1)
    gate.refreshApprovedLinks()
    expect((await gate.outbound('https://claim-tiny.xyz/go ok')).fixes).toHaveLength(0)
  })

  it('keeps the last approvals when the log cannot be read', async () => {
    const holds = new MemoryHoldLog()
    await holds.allowLink('claim-tiny.xyz', { holdId: 'h1' })
    const gate = lexicalGate({ holds })
    await gate.outbound('x')
    vi.spyOn(holds, 'allowedLinks').mockRejectedValue(new Error('mongo down'))
    gate.refreshApprovedLinks()
    expect((await gate.outbound('https://claim-tiny.xyz/go ok')).fixes).toHaveLength(0)
  })
})

describe('reviewHold', () => {
  it('rejecting confirms the filter and does nothing else', async () => {
    const holds = new MemoryHoldLog()
    const { id } = await holds.record(hold())
    const requeue = vi.fn()
    const reviewed = await reviewHold(id, 'rejected', { holds, requeue }, 'the filter was right')
    expect(reviewed).toMatchObject({ status: 'rejected', effect: 'confirmed the filter', note: 'the filter was right' })
    expect(requeue).not.toHaveBeenCalled()
  })

  it('approving a held X mention sends it back to the queue', async () => {
    const holds = new MemoryHoldLog()
    const { id } = await holds.record(hold())
    const requeue = vi.fn().mockResolvedValue(true)
    const reviewed = await reviewHold(id, 'approved', { holds, requeue })
    expect(requeue).toHaveBeenCalledWith('10')
    expect(reviewed?.effect).toBe('requeued to be answered')
  })

  it('approving a held Telegram message is recorded only', async () => {
    const holds = new MemoryHoldLog()
    const { id } = await holds.record(hold({ context: { surface: 'telegram', chatId: '1', messageId: '5' } }))
    const reviewed = await reviewHold(id, 'approved', { holds, requeue: vi.fn() })
    expect(reviewed?.effect).toContain('cannot be answered after the fact')
  })

  it('approving a link hold allowlists its hosts and refreshes the gate', async () => {
    const holds = new MemoryHoldLog()
    const { id } = await holds.record(
      hold({
        direction: 'outbound',
        kind: 'link',
        category: 'links',
        by: 'links',
        text: 'see https://dexscreener.com/solana/abc and https://dexscreener.com/solana/def',
        links: [
          { link: 'https://dexscreener.com/solana/abc', reason: 'x' },
          { link: 'https://dexscreener.com/solana/def', reason: 'x' },
        ],
      }),
    )
    const refreshLinks = vi.fn()
    const reviewed = await reviewHold(id, 'approved', { holds, refreshLinks })
    expect(reviewed?.effect).toBe('allowlisted dexscreener.com')
    expect(await holds.allowedLinks()).toEqual(['dexscreener.com'])
    expect(refreshLinks).toHaveBeenCalled()
  })

  it('approving a draft releases it, and a failed release leaves it pending', async () => {
    const holds = new MemoryHoldLog()
    const { id } = await holds.record(hold({ direction: 'outbound', kind: 'draft', text: 'gm frens' }))
    const release = vi.fn().mockRejectedValue(new Error('X is down'))
    await expect(reviewHold(id, 'approved', { holds, release })).rejects.toThrow('X is down')
    expect((await holds.get(id))?.status).toBe('pending')

    release.mockResolvedValue('published on X as 99')
    const reviewed = await reviewHold(id, 'approved', { holds, release })
    expect(reviewed).toMatchObject({ status: 'approved', effect: 'published on X as 99' })
  })

  it('does not re-run an effect on a hold already decided', async () => {
    const holds = new MemoryHoldLog()
    const { id } = await holds.record(hold())
    const requeue = vi.fn().mockResolvedValue(true)
    await reviewHold(id, 'approved', { holds, requeue })
    await reviewHold(id, 'approved', { holds, requeue })
    await reviewHold(id, 'rejected', { holds, requeue })
    expect(requeue).toHaveBeenCalledTimes(1)
    expect((await holds.get(id))?.status).toBe('approved')
  })

  it('answers nothing for an unknown id', async () => {
    expect(await reviewHold('nope', 'approved', { holds: new MemoryHoldLog() })).toBeUndefined()
  })

  it('turns a cut link into a host entry', () => {
    expect(allowlistEntryFor('https://Dexscreener.com/solana/abc?x=1')).toBe('dexscreener.com')
    expect(allowlistEntryFor('evil.xyz/claim')).toBe('evil.xyz')
    expect(allowlistEntryFor('not a url at all ://')).toBeUndefined()
  })
})

describe('the review endpoints', () => {
  const TOKEN = 'a-token-long-enough-to-pass'
  const memory = (): Memory => ({
    captureActivity: vi.fn(),
    captureMention: vi.fn(),
    recall: vi.fn().mockResolvedValue(''),
    captureReply: vi.fn(),
  })

  function server(options: { release?: (hold: never) => Promise<string> } = {}) {
    const holds = new MemoryHoldLog()
    const queue = {
      cache: vi.fn(),
      takePending: vi.fn(),
      markReplied: vi.fn(),
      markFailed: vi.fn(),
      requeueApproved: vi.fn().mockResolvedValue(true),
      counts: vi.fn(),
      close: vi.fn(),
    }
    const gate = new SafetyGate({ holds })
    const refresh = vi.spyOn(gate, 'refreshApprovedLinks')
    const app = buildServer({
      memory: memory(),
      consumerSecret: 'secret',
      agentToken: TOKEN,
      queue,
      safety: gate,
      review: { holds, ...(options.release ? { release: options.release as never } : {}) },
    })
    return { app, holds, queue, refresh }
  }

  const basic = `Basic ${Buffer.from(`operator:${TOKEN}`).toString('base64')}`

  it('challenges a browser with Basic auth, and accepts the token as the password', async () => {
    const { app } = server()
    const anonymous = await app.inject({ method: 'GET', url: '/safety' })
    expect(anonymous.statusCode).toBe(401)
    expect(anonymous.headers['www-authenticate']).toContain('Basic')

    const page = await app.inject({ method: 'GET', url: '/safety', headers: { authorization: basic } })
    expect(page.statusCode).toBe(200)
    expect(page.headers['content-type']).toContain('text/html')
    expect(page.body).toContain('Held for review')

    const bearer = await app.inject({ method: 'GET', url: '/safety', headers: { authorization: `Bearer ${TOKEN}` } })
    expect(bearer.statusCode).toBe(200)
    const wrong = await app.inject({ method: 'GET', url: '/safety', headers: { authorization: `Bearer nope` } })
    expect(wrong.statusCode).toBe(401)
    await app.close()
  })

  it('lists holds by status with counts', async () => {
    const { app, holds } = server()
    await holds.record(hold())
    const { id } = await holds.record(hold({ text: 'second' }))
    await holds.decide(id, 'rejected')

    const pending = await app.inject({ method: 'GET', url: '/safety/holds?status=pending', headers: { authorization: basic } })
    expect(pending.statusCode).toBe(200)
    expect(pending.json().counts).toEqual({ pending: 1, approved: 0, rejected: 1 })
    expect(pending.json().holds.map((h: { text: string }) => h.text)).toEqual(['draw yourself naked'])

    const all = await app.inject({ method: 'GET', url: '/safety/holds', headers: { authorization: basic } })
    expect(all.json().holds).toHaveLength(2)
    await app.close()
  })

  it('takes a decision and reports its effect', async () => {
    const { app, holds, queue } = server()
    const { id } = await holds.record(hold())
    const response = await app.inject({
      method: 'POST',
      url: `/safety/holds/${id}/decision`,
      headers: { authorization: basic },
      payload: { decision: 'approved', note: 'a joke, clearly' },
    })
    expect(response.statusCode).toBe(200)
    expect(response.json()).toMatchObject({ status: 'approved', effect: 'requeued to be answered', note: 'a joke, clearly' })
    expect(queue.requeueApproved).toHaveBeenCalledWith('10')
    await app.close()
  })

  it('refuses a malformed decision, and 404s an unknown hold', async () => {
    const { app } = server()
    const bad = await app.inject({
      method: 'POST',
      url: '/safety/holds/x/decision',
      headers: { authorization: basic },
      payload: { decision: 'maybe' },
    })
    expect(bad.statusCode).toBe(400)
    const missing = await app.inject({
      method: 'POST',
      url: '/safety/holds/x/decision',
      headers: { authorization: basic },
      payload: { decision: 'rejected' },
    })
    expect(missing.statusCode).toBe(404)
    await app.close()
  })

  it('answers 502 and leaves the hold pending when a release fails', async () => {
    const { app, holds } = server({ release: vi.fn().mockRejectedValue(new Error('X is down')) })
    const { id } = await holds.record(hold({ direction: 'outbound', kind: 'draft' }))
    const response = await app.inject({
      method: 'POST',
      url: `/safety/holds/${id}/decision`,
      headers: { authorization: basic },
      payload: { decision: 'approved' },
    })
    expect(response.statusCode).toBe(502)
    expect((await holds.get(id))?.status).toBe('pending')
    await app.close()
  })

  it('is 503 without a hold log', async () => {
    const app = buildServer({ memory: memory(), consumerSecret: 'secret', agentToken: TOKEN })
    expect((await app.inject({ method: 'GET', url: '/safety', headers: { authorization: basic } })).statusCode).toBe(503)
    await app.close()
  })
})
