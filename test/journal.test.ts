import { describe, expect, it, vi } from 'vitest'
import {
  MemoryJournal,
  redactHandles,
  redactReflection,
  summariseRecall,
  summariseResult,
} from '../src/journal/journal.js'
import { ChartJob } from '../src/jobs/chart-job.js'
import { ReplyJob } from '../src/jobs/reply-job.js'
import { ReplyHarness } from '../src/harness/reply-harness.js'
import type { MentionQueue, QueuedMention } from '../src/queue/mentions.js'
import { buildServer } from '../src/server.js'
import type { LanguageModel, Memory, XClient, XPost, XTools } from '../src/types.js'

/**
 * The journal is the public record of how the account thinks. These pin down
 * the three things that make it safe to publish: nobody else's handle leaves
 * the box, every mention gets exactly one line whatever happened to it, and a
 * journal that cannot be written never costs a reply.
 */

describe('the journal', () => {
  it('keeps newest first and never records the same turn twice', async () => {
    const journal = new MemoryJournal()
    await journal.record({ ...blank(), id: 'a', at: '2026-09-13T01:00:00Z' })
    await journal.record({ ...blank(), id: 'b', at: '2026-09-13T02:00:00Z' })
    await journal.record({ ...blank(), id: 'a', at: '2026-09-13T03:00:00Z', said: 'again' })

    const recent = await journal.recent(10)
    expect(recent.map((entry) => entry.id)).toEqual(['b', 'a'])
    expect(recent[1]?.said).toBe('')
  })

  it('blanks every handle before anything is shown', () => {
    expect(redactHandles('@someone hey @Tiny_Bot, @a1 look')).toBe('@… hey @…, @… look')
    const redacted = redactReflection({
      ...blank(),
      id: 'x',
      at: 'now',
      incoming: '@fren gm',
      said: 'gm @fren',
      thinking: 'I know @fren',
      recalled: ['spoke with @fren'],
      declined: ['tagging @other'],
      tools: [{ name: 'recall_memory', result: '@fren asked before' }],
    })
    expect(JSON.stringify(redacted)).not.toMatch(/@[A-Za-z]/)
  })

  it('cuts a memory block down to a few plain lines', () => {
    const block = [
      'ABOUT @fren:',
      '- asked about the chart twice',
      '* likes rust',
      '',
      'GENERAL CONTEXT:',
      '> ' + 'x'.repeat(300),
      'fifth line that is over the cap',
    ].join('\n')
    const lines = summariseRecall(block, 4)
    expect(lines).toHaveLength(4)
    expect(lines[0]).toBe('asked about the chart twice')
    expect(lines[2]?.length).toBe(180)
    expect(lines[2]?.endsWith('…')).toBe(true)
  })

  it('flattens a tool result to one line', () => {
    expect(summariseResult({ price: 1, note: 'a\n  b' })).toBe('{"price":1,"note":"a\\n b"}')
    expect(summariseResult('a'.repeat(200)).length).toBe(160)
    expect(summariseResult(undefined)).toBe('')
  })
})

describe('the reply job journals every mention', () => {
  function subject(dryRun: boolean, options: { post?: XPost; fail?: boolean } = {}) {
    const post = options.post ?? { id: '10', text: '@bot much higher?', authorId: '7' }
    const x = {
      findMentions: vi.fn(),
      reply: vi.fn(),
      createPost: options.fail
        ? vi.fn().mockRejectedValue(new Error('X is down'))
        : vi.fn().mockResolvedValue({ id: '99' }),
      uploadMedia: vi.fn(),
    } as XClient & Pick<XTools, 'createPost' | 'uploadMedia'>
    const memory: Memory = {
      captureActivity: vi.fn().mockResolvedValue(undefined),
      captureMention: vi.fn().mockResolvedValue(undefined),
      // The person recall, then the account's own-activity recall.
      recall: vi
        .fn()
        .mockResolvedValueOnce('- they asked about the chart before')
        .mockResolvedValueOnce(''),
      captureReply: vi.fn().mockResolvedValue(undefined),
    }
    const model: LanguageModel = {
      reply: vi.fn().mockResolvedValue({
        text: 'we are green, market is red',
        incomingTone: 'friendly',
        thinking: 'They want a number for the future. I give what already happened.',
        declined: ['a price target'],
        tools: [{ name: 'token_overview', result: 'price and volume' }],
      }),
    }
    const mentions: MentionQueue = {
      cache: vi.fn(),
      takePending: vi.fn().mockResolvedValue([
        { postId: post.id, post, status: 'pending', attempts: 0, cachedAt: '', updatedAt: '' } as QueuedMention,
      ]),
      markReplied: vi.fn().mockResolvedValue(undefined),
      markFailed: vi.fn().mockResolvedValue(undefined),
      counts: vi.fn(),
      close: vi.fn(),
    }
    const journal = new MemoryJournal()
    const job = new ReplyJob({
      x,
      harness: new ReplyHarness(memory, model),
      queue: mentions,
      logger: { info: vi.fn(), warn: vi.fn(), error: vi.fn() } as never,
      dryRun,
      maxReplies: 5,
      botUserId: async () => 'bot-id',
      journal,
    })
    return { job, journal, x }
  }

  it('records the whole turn once it has been posted', async () => {
    const it_ = subject(false)
    await it_.job.run()

    const [entry] = await it_.journal.recent(5)
    expect(entry).toMatchObject({
      id: 'mention:10',
      trigger: 'mention',
      incoming: '@bot much higher?',
      tone: 'friendly',
      recalled: ['they asked about the chart before'],
      tools: [{ name: 'token_overview', result: 'price and volume' }],
      thinking: 'They want a number for the future. I give what already happened.',
      said: 'we are green, market is red',
      declined: ['a price target'],
      outcome: 'posted',
    })
  })

  it('says dry-run when nothing was published', async () => {
    const it_ = subject(true)
    await it_.job.run()
    expect((await it_.journal.recent(1))[0]?.outcome).toBe('dry-run')
  })

  it('records a failure with nothing said', async () => {
    const it_ = subject(false, { fail: true })
    await it_.job.run()
    const [entry] = await it_.journal.recent(1)
    expect(entry).toMatchObject({ outcome: 'failed', said: '', outcomeNote: 'X is down' })
  })

  it('records the refusal to answer its own post', async () => {
    const it_ = subject(false, { post: { id: '11', text: 'me', authorId: 'bot-id' } })
    await it_.job.run()
    const [entry] = await it_.journal.recent(1)
    expect(entry?.outcome).toBe('skipped')
    expect(it_.x.createPost).not.toHaveBeenCalled()
  })

  it('still replies when the journal is broken', async () => {
    const it_ = subject(false)
    vi.spyOn(it_.journal, 'record').mockRejectedValue(new Error('mongo is down'))
    await it_.job.run()
    expect(it_.x.createPost).toHaveBeenCalled()
  })
})

describe('the chart job journals the post', () => {
  it('keeps the caption that went to post_chart, and the summary as thinking', async () => {
    const journal = new MemoryJournal()
    const job = new ChartJob({
      agent: {
        run: vi.fn().mockResolvedValue({
          output: 'The read: the level held.',
          steps: [
            { tool: 'technical_analysis', input: {}, result: 'support at 0.00056' },
            { tool: 'post_chart', input: { chartId: 'chart_1', text: 'level held, volume came in' } },
          ],
        }),
      },
      symbol: 'TINY',
      timeframe: '1h',
      logger: { info: vi.fn(), error: vi.fn() },
      now: () => Date.UTC(2026, 8, 13, 15),
      journal,
    })
    await job.run()

    const [entry] = await journal.recent(1)
    expect(entry).toMatchObject({
      id: 'chart:2026-09-13',
      trigger: 'chart',
      said: 'level held, volume came in',
      thinking: 'The read: the level held.',
      tools: [
        { name: 'technical_analysis', result: 'support at 0.00056' },
        { name: 'post_chart', result: '' },
      ],
      outcome: 'posted',
    })
  })
})

describe('GET /mind', () => {
  it('serves the journal, redacted, with the queue counts', async () => {
    const journal = new MemoryJournal()
    await journal.record({ ...blank(), incoming: '@fren gm', said: 'gm @fren' })
    const app = buildServer({
      memory: {
        captureActivity: vi.fn(),
        captureMention: vi.fn(),
        recall: vi.fn(),
        captureReply: vi.fn(),
      },
      consumerSecret: 'secret',
      queue: {
        cache: vi.fn(),
        takePending: vi.fn(),
        markReplied: vi.fn(),
        markFailed: vi.fn(),
        counts: vi.fn().mockResolvedValue({ pending: 2, replied: 40, failed: 1, skipped: 3 }),
        close: vi.fn(),
      },
      mind: { journal, status: { mode: 'langgraph', model: 'flash', dryRun: false } },
    })

    const response = await app.inject({ method: 'GET', url: '/mind' })
    expect(response.statusCode).toBe(200)
    const body = response.json()
    expect(body.source).toBe('live')
    expect(body.status).toEqual({
      mode: 'langgraph',
      model: 'flash',
      dryRun: false,
      queue: { pending: 2, done: 40, failed: 1 },
    })
    expect(body.reflections[0].incoming).toBe('@… gm')
    expect(body.reflections[0].said).toBe('gm @…')
    await app.close()
  })

  it('is 503 without a journal', async () => {
    const app = buildServer({
      memory: { captureActivity: vi.fn(), captureMention: vi.fn(), recall: vi.fn(), captureReply: vi.fn() },
      consumerSecret: 'secret',
    })
    expect((await app.inject({ method: 'GET', url: '/mind' })).statusCode).toBe(503)
    await app.close()
  })
})

function blank() {
  return {
    trigger: 'mention' as const,
    incoming: '',
    tone: 'neutral' as const,
    recalled: [],
    tools: [],
    thinking: '',
    said: '',
    declined: [],
    outcome: 'posted' as const,
  }
}
