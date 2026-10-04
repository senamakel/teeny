import { describe, expect, it, vi } from 'vitest'
import { capPack, CortexMemory } from '../src/memory/cortex.js'

/**
 * The client against a fake transport. Every other test stubs `Memory`
 * wholesale, so nothing checked what actually goes over the wire: the scope,
 * the person filter, the deadline. These do.
 */
type Call = { url: string; body: Record<string, unknown> }

function transport(answer: (call: Call) => Response | Promise<Response>) {
  const calls: Call[] = []
  const fake = vi.fn(async (url: string | URL, init?: RequestInit) => {
    const call = { url: String(url), body: init?.body ? (JSON.parse(String(init.body)) as Record<string, unknown>) : {} }
    calls.push(call)
    if (init?.signal) {
      // Honour the abort the way fetch does, so a deadline can be observed.
      return Promise.race([
        Promise.resolve(answer(call)),
        new Promise<Response>((_, reject) =>
          init.signal!.addEventListener('abort', () => reject(new DOMException('aborted', 'AbortError'))),
        ),
      ])
    }
    return answer(call)
  })
  return { fetch: fake as unknown as typeof fetch, calls }
}

const pack = (block: string) => new Response(JSON.stringify({ context_block: block }), { status: 200 })
const options = { apiUrl: 'http://cortex.test', actor: 'agent:teeny', bearer: 't' }

describe('CortexMemory.recall', () => {
  it('asks a holistic pack in the configured scope, and one filtered to the person', async () => {
    const { fetch, calls } = transport((call) =>
      pack(JSON.stringify(call.body).includes('observed_actor') ? 'they asked about the roadmap' : 'the project ships weekly'),
    )
    const memory = new CortexMemory({ ...options, fetch }, 'agent:teeny')

    const text = await memory.recall('what about the roadmap?', { person: { id: 'user:x-1', username: 'ana' } })

    expect(calls).toHaveLength(2)
    expect(calls.every((call) => call.url.endsWith('/v1/recall'))).toBe(true)
    expect(calls.every((call) => call.body.scope === 'agent:teeny' && call.body.view === 'holistic')).toBe(true)
    const filtered = calls.find((call) => call.body.filters)
    expect(filtered?.body.filters).toEqual({ metadata: { observed_actor: 'user:x-1' } })
    expect(text).toBe('ABOUT @ana:\nthey asked about the roadmap\n\nGENERAL CONTEXT:\nthe project ships weekly')
  })

  it('matches every id a linked person is known by', async () => {
    const { fetch, calls } = transport(() => pack(''))
    const memory = new CortexMemory({ ...options, fetch }, 'agent:teeny')

    await memory.recall('hi', { person: { id: 'user:x-1', aliases: ['user:telegram-9'] } })

    const filtered = calls.find((call) => call.body.filters)
    expect(filtered?.body.filters).toEqual({ metadata: { observed_actor: ['user:x-1', 'user:telegram-9'] } })
  })

  it('keeps the general context when only the person pass fails', async () => {
    const { fetch } = transport((call) =>
      call.body.filters ? new Response('{"error_code":"DB_UNAVAILABLE"}', { status: 503 }) : pack('the project ships weekly'),
    )
    const memory = new CortexMemory({ ...options, fetch }, 'agent:teeny')

    await expect(memory.recall('q', { person: { id: 'user:x-1' } })).resolves.toBe('GENERAL CONTEXT:\nthe project ships weekly')
  })

  it('fails when both passes fail, so the caller can say memory was unavailable', async () => {
    const { fetch } = transport(() => new Response('{"error_code":"DB_UNAVAILABLE"}', { status: 503 }))
    const memory = new CortexMemory({ ...options, fetch }, 'agent:teeny')

    await expect(memory.recall('q', { person: { id: 'user:x-1' } })).rejects.toThrow()
  })

  it('gives up on a recall at its own deadline rather than the SDK minute', async () => {
    const { fetch } = transport(() => new Promise<Response>(() => undefined))
    const memory = new CortexMemory({ ...options, fetch }, 'agent:teeny', { recallMs: 30 })

    const started = Date.now()
    await expect(memory.recall('q')).rejects.toThrow(/timed out after 30ms/)
    expect(Date.now() - started).toBeLessThan(2_000)
  })
})

describe('CortexMemory captures', () => {
  it('files a mention under the person, with a versioned idempotency key', async () => {
    const { fetch, calls } = transport(() => new Response('{}', { status: 202 }))
    const memory = new CortexMemory({ ...options, fetch }, 'agent:teeny')

    await memory.captureMention({ id: '42', text: 'gm', authorId: '7', authorUsername: 'ana', conversationId: '40' })

    expect(calls[0]?.url).toContain('/v1/experience')
    const item = JSON.stringify(calls[0]?.body)
    expect(item).toContain('x:mention:2:42')
    expect(item).toContain('user:x-7')
    expect(item).toContain('thread:40')
  })

  it('treats an idempotency conflict as already stored', async () => {
    const { fetch } = transport(
      () => new Response(JSON.stringify({ error_code: 'IDEMPOTENCY_CONFLICT', message: 'IDEMPOTENCY_CONFLICT' }), { status: 409 }),
    )
    const memory = new CortexMemory({ ...options, fetch }, 'agent:teeny')

    await expect(memory.captureMention({ id: '42', text: 'gm', authorId: '7' })).resolves.toBeUndefined()
  })
})

describe('capPack', () => {
  it('cuts an over-budget pack at a memory boundary', () => {
    const items = Array.from({ length: 1_000 }, (_, index) => `memory ${index} `.repeat(20))
    const capped = capPack(items.join('\n\n---\n\n'), 1_000)

    expect(capped.length).toBeLessThanOrEqual(1_000)
    expect(capped.startsWith('memory 0 ')).toBe(true)
    // Whole items only: the last one kept ends where an item ends.
    expect(capped.split('\n\n---\n\n').every((item) => item === items[Number(item.split(' ')[1])])).toBe(true)
  })

  it('leaves a pack inside its budget alone, and hard-cuts a single giant item', () => {
    expect(capPack('short', 100)).toBe('short')
    expect(capPack('x'.repeat(500), 100)).toHaveLength(100)
  })

  it('applies to what recall returns', async () => {
    const { fetch } = transport(() => pack(Array.from({ length: 5_000 }, () => 'a like event').join('\n\n---\n\n')))
    const memory = new CortexMemory({ ...options, fetch }, 'agent:teeny')

    const text = await memory.recall('q')
    expect(text.length).toBeLessThan(1_200 * 6 + 100)
  })
})
