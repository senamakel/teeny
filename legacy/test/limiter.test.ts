import { describe, expect, it, vi } from 'vitest'
import { createLimiter } from '../src/market/limiter.js'

describe('the provider rate limiter', () => {
  it('runs one call at a time, in order', async () => {
    const limit = createLimiter(0)
    const order: number[] = []
    const slow = async (n: number, ms: number): Promise<void> => {
      await new Promise((resolve) => setTimeout(resolve, ms))
      order.push(n)
    }

    await Promise.all([limit(() => slow(1, 20)), limit(() => slow(2, 1)), limit(() => slow(3, 1))])
    expect(order).toEqual([1, 2, 3])
  })

  it('keeps a gap between calls', async () => {
    vi.useFakeTimers()
    try {
      const limit = createLimiter(1_000)
      const started: number[] = []
      const work = (): Promise<void> => {
        started.push(Date.now())
        return Promise.resolve()
      }

      const all = Promise.all([limit(work), limit(work), limit(work)])
      await vi.advanceTimersByTimeAsync(5_000)
      await all

      expect(started).toHaveLength(3)
      expect((started[1] as number) - (started[0] as number)).toBeGreaterThanOrEqual(1_000)
      expect((started[2] as number) - (started[1] as number)).toBeGreaterThanOrEqual(1_000)
    } finally {
      vi.useRealTimers()
    }
  })

  /** One rejection must not wedge everything queued behind it. */
  it('keeps going after a call fails', async () => {
    const limit = createLimiter(0)
    const failed = limit(() => Promise.reject(new Error('429')))

    await expect(failed).rejects.toThrow('429')
    await expect(limit(() => Promise.resolve('ok'))).resolves.toBe('ok')
  })

  it('surfaces the result and the error to the right caller', async () => {
    const limit = createLimiter(0)
    const results = await Promise.allSettled([
      limit(() => Promise.resolve('first')),
      limit(() => Promise.reject(new Error('second failed'))),
      limit(() => Promise.resolve('third')),
    ])

    expect(results.map((result) => result.status)).toEqual(['fulfilled', 'rejected', 'fulfilled'])
  })
})
