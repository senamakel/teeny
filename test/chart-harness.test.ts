import { describe, expect, it, vi } from 'vitest'
import { captureChartPost, ChartHarness } from '../src/harness/chart-harness.js'
import type { Agent, Memory } from '../src/types.js'

function memory(recall = vi.fn().mockResolvedValue('')): Memory {
  return {
    captureActivity: vi.fn(),
    captureMention: vi.fn(),
    recall,
    captureReply: vi.fn(),
  } as unknown as Memory
}

function agent(): Agent {
  return { run: vi.fn().mockResolvedValue({ output: 'done', steps: [] }) }
}

/** The task the harness actually handed the agent. */
function taskOf(inner: Agent): string {
  const call = (inner.run as ReturnType<typeof vi.fn>).mock.calls[0] as [{ task: string }]
  return call[0].task
}

describe('chart harness', () => {
  it('fetches history and community every run instead of hoping the model asks', async () => {
    const recall = vi
      .fn()
      .mockResolvedValueOnce('yesterday you posted about the 0.0007 reclaim')
      .mockResolvedValueOnce('@someone has been asking about the burn')
    const inner = agent()
    await new ChartHarness(memory(recall), inner, { symbol: 'TINY' }).run({ task: 'post it' })

    expect(recall).toHaveBeenCalledTimes(2)
    const task = taskOf(inner)
    expect(task).toContain('yesterday you posted about the 0.0007 reclaim')
    expect(task).toContain('@someone has been asking about the burn')
    expect(task).toContain('THE TINY COMMUNITY')
  })

  it('asks about the community by name, not in general', async () => {
    const recall = vi.fn().mockResolvedValue('')
    await new ChartHarness(memory(recall), agent(), { symbol: 'TINY' }).run({ task: 'post it' })
    expect(recall.mock.calls.map(String).join(' ')).toContain('TINY holders')
  })

  /** A memory outage is a worse post, not a missing day. */
  it('still runs when recall fails', async () => {
    const recall = vi.fn().mockRejectedValue(new Error('401 MISSING_TOKEN'))
    const inner = agent()
    const result = await new ChartHarness(memory(recall), inner, { symbol: 'TINY' }).run({
      task: 'post it',
    })

    expect(result.output).toBe('done')
    expect(taskOf(inner)).toContain('nothing recalled')
  })

  it('tells the agent to treat an empty history as a first post rather than improvise', async () => {
    const inner = agent()
    await new ChartHarness(memory(), inner, { symbol: 'TINY' }).run({ task: 'post it' })
    expect(taskOf(inner)).toContain('do not pretend otherwise')
  })

  it('carries the thread id through so a day stays one conversation', async () => {
    const inner = agent()
    await new ChartHarness(memory(), inner, { symbol: 'TINY' }).run({
      task: 'post it',
      threadId: 'chart:2026-09-09',
    })
    expect(inner.run).toHaveBeenCalledWith(
      expect.objectContaining({ threadId: 'chart:2026-09-09' }),
    )
  })
})

describe('capturing the post', () => {
  it('stores the text itself, not a paraphrase of it', async () => {
    const store = memory()
    await captureChartPost(store, 'TINY', {
      text: 'the level held',
      postedId: '123',
      chartId: 'chart_1',
      dryRun: false,
    })

    expect(store.captureActivity).toHaveBeenCalledWith(
      'chart_post',
      '123',
      expect.objectContaining({ text: 'the level held', symbol: 'TINY', published: true }),
    )
  })

  it('marks a rehearsal as unpublished so it cannot poison tomorrow', async () => {
    const store = memory()
    await captureChartPost(store, 'TINY', { text: 'draft', dryRun: true })

    const call = (store.captureActivity as ReturnType<typeof vi.fn>).mock.calls[0] ?? []
    expect(call[0]).toBe('chart_post')
    expect(call[1]).toMatch(/^dry-run:/)
    expect(call[2]).toMatchObject({ published: false })
  })
})
