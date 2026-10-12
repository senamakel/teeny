import { describe, expect, it, vi } from 'vitest'
import { checkCortex, reportCortexHealth } from '../src/memory/health.js'

function answering(body: unknown, status = 200): typeof fetch {
  return vi.fn().mockResolvedValue(new Response(JSON.stringify(body), { status })) as unknown as typeof fetch
}

describe('the memory store health check', () => {
  it('reads an embedding outage out of the readiness report', async () => {
    // The shape robot1 reported on 2026-10-04, when the router had dropped the
    // embedding ladder: up, ready, and unable to embed anything.
    const health = await checkCortex('http://cortex.test', {
      fetch: answering({
        ready: true,
        degraded: true,
        embedding_provider: 'openai-http:vectors:1024',
        checks: { embeddings: { ok: false, ok_count: 0, fail_count: 28_517, last_error: 'unknown ladder vectors' } },
      }),
    })

    expect(health).toMatchObject({
      reachable: true,
      degraded: true,
      embeddings: { ok: false, okCount: 0, failCount: 28_517, lastError: 'unknown ladder vectors' },
    })
  })

  it('warns on an outage and is quiet about a healthy store', async () => {
    const logger = { info: vi.fn(), warn: vi.fn() }
    await reportCortexHealth('http://cortex.test', logger, {
      fetch: answering({ degraded: true, checks: { embeddings: { ok: false } } }),
    })
    expect(logger.warn).toHaveBeenCalledWith(expect.anything(), expect.stringContaining('cannot embed'))

    logger.warn.mockClear()
    await reportCortexHealth('http://cortex.test', logger, {
      fetch: answering({ degraded: false, checks: { embeddings: { ok: true, ok_count: 149 } } }),
    })
    expect(logger.warn).not.toHaveBeenCalled()
    expect(logger.info).toHaveBeenCalledWith(expect.anything(), 'memory: the store is healthy')
  })

  it('reports an unreachable store rather than throwing', async () => {
    const health = await checkCortex('http://cortex.test', {
      fetch: vi.fn().mockRejectedValue(new Error('ECONNREFUSED')) as unknown as typeof fetch,
    })
    expect(health).toMatchObject({ reachable: false, degraded: true, error: 'ECONNREFUSED' })
  })
})
