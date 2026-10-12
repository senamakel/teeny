import { describe, expect, it, vi } from 'vitest'
import { ChartJob } from '../src/jobs/chart-job.js'
import type { Agent } from '../src/types.js'

function logger() {
  return { info: vi.fn(), error: vi.fn() }
}

function agent(): Agent {
  return { run: vi.fn().mockResolvedValue({ output: 'posted', steps: [{ tool: 'post_chart', input: {} }] }) }
}

describe('chart job', () => {
  it('runs the agent and reports what it called', async () => {
    const chartAgent = agent()
    const log = logger()
    const job = new ChartJob({ agent: chartAgent, symbol: 'TINY', timeframe: '1h', logger: log })

    expect(await job.run()).toBe(true)
    expect(chartAgent.run).toHaveBeenCalledWith(
      expect.objectContaining({ task: expect.stringContaining('TINY') }),
    )
    expect(log.info).toHaveBeenCalledWith(
      expect.objectContaining({ tools: ['post_chart'] }),
      'chart job: done',
    )
  })

  /**
   * A container that restarts near the cron minute fires the schedule again.
   * Two identical chart posts an hour apart is the repetitive-publishing shape
   * that cost this account its reach once already, so the guard is in the job.
   */
  it('declines to post twice inside the interval', async () => {
    const chartAgent = agent()
    let now = 1_000_000
    const job = new ChartJob({
      agent: chartAgent,
      symbol: 'TINY',
      timeframe: '1h',
      logger: logger(),
      now: () => now,
    })

    expect(await job.run()).toBe(true)
    now += 60 * 60 * 1_000
    expect(await job.run()).toBe(false)
    expect(chartAgent.run).toHaveBeenCalledTimes(1)
  })

  it('posts again once a day has gone by', async () => {
    const chartAgent = agent()
    let now = 1_000_000
    const job = new ChartJob({
      agent: chartAgent,
      symbol: 'TINY',
      timeframe: '1h',
      logger: logger(),
      now: () => now,
    })

    await job.run()
    now += 21 * 60 * 60 * 1_000
    expect(await job.run()).toBe(true)
    expect(chartAgent.run).toHaveBeenCalledTimes(2)
  })

  it('runs anyway when a human forces it', async () => {
    const chartAgent = agent()
    const job = new ChartJob({
      agent: chartAgent,
      symbol: 'TINY',
      timeframe: '1h',
      logger: logger(),
      now: () => 1_000_000,
    })

    await job.run()
    expect(await job.run({ force: true })).toBe(true)
  })

  it('keys the thread on the day, so one day is one conversation', async () => {
    const chartAgent = agent()
    const job = new ChartJob({
      agent: chartAgent,
      symbol: 'TINY',
      timeframe: '1h',
      logger: logger(),
      now: () => Date.parse('2026-09-08T15:00:00Z'),
    })

    await job.run()
    expect(chartAgent.run).toHaveBeenCalledWith(
      expect.objectContaining({ threadId: 'chart:2026-09-08' }),
    )
  })
})
