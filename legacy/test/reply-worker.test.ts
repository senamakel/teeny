import type { FastifyBaseLogger } from 'fastify'
import { describe, expect, it, vi } from 'vitest'
import { ReplyWorker } from '../src/jobs/reply-worker.js'
import type { ReplyJob } from '../src/jobs/reply-job.js'

/**
 * The worker exists because a webhook delivery used to sit in the queue until
 * the next cron tick: the mention arrived in seconds and the answer was up to
 * five minutes behind a conversation still in progress.
 */
function logger(): FastifyBaseLogger {
  return { info: vi.fn(), warn: vi.fn(), error: vi.fn() } as unknown as FastifyBaseLogger
}

function job(runs: Array<boolean | Error>): ReplyJob & { run: ReturnType<typeof vi.fn> } {
  const run = vi.fn()
  for (const outcome of runs) {
    if (outcome instanceof Error) run.mockRejectedValueOnce(outcome)
    else run.mockResolvedValueOnce(outcome)
  }
  run.mockResolvedValue(false)
  return { run } as unknown as ReplyJob & { run: ReturnType<typeof vi.fn> }
}

describe('ReplyWorker', () => {
  it('comes straight back for more while there is work, without waiting', async () => {
    const it_ = job([true, true, true])
    const worker = new ReplyWorker({ job: it_, logger: logger(), idleMs: 50_000 })

    worker.start()
    // Three batches back to back. With a cron, or with an idle wait between
    // them, only one could have happened in this window.
    await vi.waitFor(() => expect(it_.run.mock.calls.length).toBeGreaterThanOrEqual(3))
    await worker.stop()
  })

  it('waits when the queue is empty rather than spinning', async () => {
    const it_ = job([false])
    const worker = new ReplyWorker({ job: it_, logger: logger(), idleMs: 10_000 })

    worker.start()
    await vi.waitFor(() => expect(it_.run).toHaveBeenCalled())
    const after = it_.run.mock.calls.length
    await new Promise((resolve) => setTimeout(resolve, 50))

    expect(it_.run.mock.calls.length).toBe(after)
    await worker.stop()
  })

  // A model outage used to be a logged line every five minutes. In a loop it
  // would be a hot spin, so a failure backs off rather than retrying instantly.
  it('backs off after a failure instead of hammering', async () => {
    const it_ = job([new Error('ladder is down')])
    const log = logger()
    const worker = new ReplyWorker({ job: it_, logger: log, idleMs: 1, errorMs: 10_000 })

    worker.start()
    await vi.waitFor(() => expect(log.error).toHaveBeenCalled())
    const after = it_.run.mock.calls.length
    await new Promise((resolve) => setTimeout(resolve, 50))

    expect(it_.run.mock.calls.length).toBe(after)
    await worker.stop()
  })

  it('lets the batch in flight finish before it stops', async () => {
    let finished = false
    const it_ = {
      run: vi.fn().mockImplementation(async () => {
        await new Promise((resolve) => setTimeout(resolve, 30))
        finished = true
        return false
      }),
    } as unknown as ReplyJob
    const worker = new ReplyWorker({ job: it_, logger: logger(), idleMs: 10_000 })

    worker.start()
    await new Promise((resolve) => setTimeout(resolve, 5))
    await worker.stop()

    expect(finished).toBe(true)
  })
})
