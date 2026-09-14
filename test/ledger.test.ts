import { mkdtemp, mkdir, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { AIMessage, HumanMessage, SystemMessage, ToolMessage, type BaseMessage } from '@langchain/core/messages'
import { describe, expect, it, vi } from 'vitest'
import { LedgerCallbacks, ledgerMessages, outputText, reasoningText } from '../src/ledger/callbacks.js'
import { recordCompletion } from '../src/ledger/completion.js'
import { BrainReader, tailLines } from '../src/ledger/brain.js'
import {
  MemoryLedger,
  redactInference,
  sha256,
  tokensFromUsage,
  zeroTokens,
} from '../src/ledger/ledger.js'
import { buildServer } from '../src/server.js'

/**
 * The ledger is the proof under the journal: a row per model call, and a
 * count that only goes up. These pin down what makes it publishable -- the
 * system prompt never leaves, handles never leave, a retried write counts
 * once -- and what makes the counter honest: every surface's usage lands in
 * it, including the brain's, in the same units.
 */

describe('the inference ledger', () => {
  it('sums every row into its lane and its source, and never counts a row twice', async () => {
    const ledger = new MemoryLedger()
    await ledger.record({ id: 'a', lane: 'mascot', source: 'reply', model: 'm', tokens: tokens(100, 20) })
    await ledger.record({ id: 'b', lane: 'mascot', source: 'chart', model: 'm', tokens: tokens(50, 5) })
    await ledger.record({ id: 'c', lane: 'brain', source: 'scout', model: 'm', tokens: tokens(7, 3) })
    await ledger.record({ id: 'a', lane: 'mascot', source: 'reply', model: 'm', tokens: tokens(999, 999) })

    const totals = await ledger.totals()
    expect(totals.lanes.mascot.calls).toBe(2)
    expect(totals.lanes.mascot.tokens.total).toBe(175)
    expect(totals.lanes.brain).toMatchObject({ calls: 1, tokens: { input: 7, output: 3, total: 10 } })
    expect(totals.sources['mascot/reply']).toMatchObject({ calls: 1, tokens: { total: 120 } })
    expect(totals.sources['mascot/chart']?.calls).toBe(1)
    expect(totals.lanes.mascot.first).toBeDefined()
  })

  it('lists newest first and filters by lane', async () => {
    const ledger = new MemoryLedger()
    await ledger.record({ id: 'old', at: '2026-09-14T01:00:00Z', lane: 'brain', source: 'scout', model: 'm', tokens: zeroTokens() })
    await ledger.record({ id: 'new', at: '2026-09-14T02:00:00Z', lane: 'mascot', source: 'reply', model: 'm', tokens: zeroTokens() })
    expect((await ledger.recent(10)).map((row) => row.id)).toEqual(['new', 'old'])
    expect((await ledger.recent(10, { lane: 'brain' })).map((row) => row.id)).toEqual(['old'])
    expect((await ledger.recent(10, { before: '2026-09-14T02:00:00Z' })).map((row) => row.id)).toEqual(['old'])
  })

  it('reads an OpenAI usage block, details and all, and tolerates their absence', () => {
    expect(
      tokensFromUsage({
        prompt_tokens: 120,
        completion_tokens: 40,
        total_tokens: 160,
        prompt_tokens_details: { cached_tokens: 100 },
        completion_tokens_details: { reasoning_tokens: 25 },
      }),
    ).toEqual({ input: 120, output: 40, reasoning: 25, cached: 100, total: 160 })
    expect(tokensFromUsage({ prompt_tokens: 3, completion_tokens: 4 })).toEqual({
      input: 3, output: 4, reasoning: 0, cached: 0, total: 7,
    })
    expect(tokensFromUsage(undefined)).toEqual(zeroTokens())
    // LangChain's own shape, as a fallback.
    expect(
      tokensFromUsage({ input_tokens: 10, output_tokens: 5, total_tokens: 15, output_token_details: { reasoning: 2 }, input_token_details: { cache_read: 4 } }),
    ).toEqual({ input: 10, output: 5, reasoning: 2, cached: 4, total: 15 })
  })

  it('blanks handles in what it shows', () => {
    const row = redactInference({
      id: 'x', at: 'now', lane: 'mascot', source: 'reply', model: 'm', tokens: zeroTokens(),
      input: [{ role: 'user', text: 'from @fren: gm' }],
      reasoning: 'they are @fren',
      output: 'gm @fren',
    })
    expect(row.input?.[0]?.text).toBe('from @…: gm')
    expect(row.reasoning).toBe('they are @…')
    expect(row.output).toBe('gm @…')
  })
})

describe('the graph hook', () => {
  const system = new SystemMessage('SECRET PROMPT: you are Tiny')
  const task = new HumanMessage('gm @fren what is medulla')
  const asked = new AIMessage({ content: '', tool_calls: [{ id: 'c1', name: 'recall_memory', args: { query: 'medulla' } }] })
  const answered = new ToolMessage({ tool_call_id: 'c1', name: 'recall_memory', content: 'Medulla manages agents' })

  it('keeps everything but the system prompt, and commits to the prompt by digest', async () => {
    const ledger = new MemoryLedger()
    const hook = new LedgerCallbacks({ ledger, lane: 'mascot', source: 'reply', model: 'gpt-x' })
    const reply = new AIMessage({
      content: 'medulla runs the agents',
      id: 'chatcmpl-123',
      additional_kwargs: { reasoning_content: 'a real question, straight answer' },
      response_metadata: {
        model_name: 'gpt-x-2026',
        usage: { prompt_tokens: 800, completion_tokens: 30, total_tokens: 830, completion_tokens_details: { reasoning_tokens: 12 } },
      },
    })
    await hook.handleChatModelStart({} as never, [[system, task, asked, answered]], 'run-1')
    await hook.handleLLMEnd({ generations: [[{ text: '', message: reply }]] } as never, 'run-1')

    const [row] = await ledger.recent(1)
    expect(row).toBeDefined()
    expect(row!.source).toBe('reply')
    expect(row!.model).toBe('gpt-x-2026')
    expect(row!.responseId).toBe('chatcmpl-123')
    expect(row!.tokens).toEqual({ input: 800, output: 30, reasoning: 12, cached: 0, total: 830 })
    expect(row!.reasoning).toBe('a real question, straight answer')
    expect(row!.output).toBe('medulla runs the agents')
    expect(row!.input).toEqual([
      { role: 'user', text: 'gm @fren what is medulla' },
      { role: 'assistant', tool: 'recall_memory', text: '→ recall_memory({"query":"medulla"})' },
      { role: 'tool', tool: 'recall_memory', text: 'Medulla manages agents' },
    ])
    expect(JSON.stringify(row)).not.toContain('SECRET PROMPT')
    expect(row!.promptDigest).toBe(sha256('SECRET PROMPT: you are Tiny'))
    expect(row!.digest).toMatch(/^[0-9a-f]{64}$/)
  })

  it('writes nothing for a call that errored, and never lets a broken ledger throw', async () => {
    const ledger = new MemoryLedger()
    const hook = new LedgerCallbacks({ ledger, lane: 'mascot', source: 'reply', model: 'm' })
    await hook.handleChatModelStart({} as never, [[task]], 'run-2')
    await hook.handleLLMError(new Error('boom'), 'run-2')
    await hook.handleLLMEnd({ generations: [[{ text: '', message: new AIMessage('late') }]] } as never, 'run-2')
    expect(await ledger.recent(10)).toEqual([])

    const broken = { record: vi.fn().mockRejectedValue(new Error('mongo down')), recent: vi.fn(), totals: vi.fn() }
    const log = vi.fn()
    const quiet = new LedgerCallbacks({ ledger: broken, lane: 'mascot', source: 'reply', model: 'm', log })
    await quiet.handleChatModelStart({} as never, [[task]], 'run-3')
    await expect(
      quiet.handleLLMEnd({ generations: [[{ text: '', message: new AIMessage('ok') }]] } as never, 'run-3'),
    ).resolves.toBeUndefined()
    expect(log).toHaveBeenCalled()
  })

  it('renders tool calls as output and reads reasoning off whichever field carries it', () => {
    expect(outputText(asked)).toBe('→ recall_memory({"query":"medulla"})')
    expect(reasoningText(new AIMessage({ content: '', additional_kwargs: { reasoning: 'plain' } }))).toBe('plain')
    expect(
      reasoningText(new AIMessage({ content: [{ type: 'reasoning', text: 'in parts' } as never, { type: 'text', text: 'hi' }] })),
    ).toBe('in parts')
    expect(reasoningText(new AIMessage('none'))).toBeUndefined()
  })

  it('keeps the task and the tail of a long tool trail', () => {
    const trail: BaseMessage[] = [system, task]
    for (let i = 0; i < 20; i += 1) {
      trail.push(new AIMessage({ content: '', tool_calls: [{ id: `c${i}`, name: 'web_search', args: { i } }] }))
      trail.push(new ToolMessage({ tool_call_id: `c${i}`, name: 'web_search', content: `result ${i}` }))
    }
    const rows = ledgerMessages(trail)
    expect(rows).toHaveLength(12)
    expect(rows[0]).toEqual({ role: 'user', text: 'gm @fren what is medulla' })
    expect(rows[rows.length - 1]?.text).toBe('result 19')
  })
})

describe('a plain chat completion', () => {
  it('lands in the ledger with the same shape as a graph step', async () => {
    const ledger = new MemoryLedger()
    await recordCompletion(ledger, {
      source: 'vision',
      model: 'eyes',
      messages: [
        { role: 'system', content: 'describe pictures' },
        { role: 'user', content: [{ type: 'text', text: 'what is this' }, { type: 'image_url', image_url: { url: 'data:...' } }] },
      ],
      response: { id: 'resp-1', model: 'eyes-v2', choices: [{ message: { content: 'a smol robot' } }], usage: { prompt_tokens: 500, completion_tokens: 12 } },
      startedAt: Date.now() - 5,
    })
    const [row] = await ledger.recent(1)
    expect(row).toMatchObject({
      lane: 'mascot', source: 'vision', model: 'eyes-v2', responseId: 'resp-1', output: 'a smol robot',
      tokens: { input: 500, output: 12, total: 512 },
    })
    expect(row!.input).toEqual([{ role: 'user', text: 'what is this[image]' }])
    expect(JSON.stringify(row)).not.toContain('describe pictures')
    expect(row!.promptDigest).toBe(sha256('describe pictures'))
  })

  it('is a no-op without a ledger', async () => {
    await expect(
      recordCompletion(undefined, { source: 'x', model: 'm', messages: [], response: {}, startedAt: 0 }),
    ).resolves.toBeUndefined()
  })
})

describe('the brain, read from its checkout', () => {
  async function checkout(): Promise<string> {
    const dir = await mkdtemp(join(tmpdir(), 'brain-'))
    await mkdir(join(dir, 'company', 'teeny', 'agents'), { recursive: true })
    await mkdir(join(dir, 'company', 'teeny', 'workflows'), { recursive: true })
    await mkdir(join(dir, 'companies', 'teeny'), { recursive: true })
    await writeFile(join(dir, 'company', 'teeny', 'company.toml'), '[company]\nname = "Teeny"\noutput = "a mind"\n')
    await writeFile(join(dir, 'company', 'teeny', 'agents', 'scout.toml'), 'role = "Scout"\ndescription = "watches"\n')
    await writeFile(join(dir, 'company', 'teeny', 'agents', 'curator.toml'), 'role = "Curator"\ndescription = "edits"\ntier = "orchestrator"\n')
    await writeFile(
      join(dir, 'company', 'teeny', 'workflows', 'reflect.toml'),
      [
        'id = "reflect"', 'name = "Reflect"', 'description = "look, learn, revise"',
        '[[node]]', 'id = "daily"', 'kind = "trigger"', 'name = "Every morning"', 'schedule = "0 6 * * *"',
        '[[node]]', 'id = "look"', 'kind = "agent"', 'name = "Look"', 'agent = "scout"',
        '[[edge]]', 'from = "daily"', 'to = "look"', 'label = "go"',
      ].join('\n'),
    )
    return dir
  }

  it('serves the roster, the workflow graph and the latest runs, one row per run', async () => {
    const dir = await checkout()
    const run = (id: string, status: string, created: number) =>
      JSON.stringify({ id, company: 'teeny', agentId: 'scout', attempt: 1, status, createdAtMillis: created, usage: { input: 10, output: 2 } })
    await writeFile(join(dir, 'companies', 'teeny', 'runs.jsonl'), [run('r1', 'running', 1), run('r2', 'pending', 2), run('r1', 'succeeded', 1), ''].join('\n'))
    await writeFile(
      join(dir, 'companies', 'teeny', 'run-steps.jsonl'),
      [
        JSON.stringify({ runId: 'r1', stepSeq: 1, atMillis: 5, step: { kind: 'thinking', status: 'ok', label: 'reading the record' } }),
        JSON.stringify({ runId: 'zzz', stepSeq: 1, atMillis: 6, step: { kind: 'note', status: 'ok', label: 'not ours' } }),
        '{torn',
      ].join('\n'),
    )
    const brain = new BrainReader({ dir })
    expect(await brain.exists()).toBe(true)
    const snapshot = await brain.snapshot()
    expect(snapshot.company).toEqual({ name: 'Teeny', output: 'a mind', slug: 'teeny' })
    expect(snapshot.agents).toEqual([
      { id: 'curator', role: 'Curator', description: 'edits', tier: 'orchestrator' },
      { id: 'scout', role: 'Scout', description: 'watches' },
    ])
    expect(snapshot.workflows[0]).toMatchObject({
      id: 'reflect',
      nodes: [
        { id: 'daily', kind: 'trigger', name: 'Every morning', schedule: '0 6 * * *' },
        { id: 'look', kind: 'agent', name: 'Look', agent: 'scout' },
      ],
      edges: [{ from: 'daily', to: 'look', label: 'go' }],
    })
    expect(snapshot.runs.map((r) => [r.id, r.status])).toEqual([['r2', 'pending'], ['r1', 'succeeded']])
    expect(snapshot.steps).toEqual([
      { runId: 'r1', stepSeq: 1, atMillis: 5, step: { kind: 'thinking', status: 'ok', label: 'reading the record' } },
    ])
  })

  it('folds usage samples into the brain lane by agent, and only once', async () => {
    const dir = await checkout()
    const sample = (at: number, agent: string, input: number, output: number, kind = 'inference') =>
      JSON.stringify({ atMillis: at, agent, provider: 'managed', model: 'flash', inputTokens: input, outputTokens: output, cachedInputTokens: 0, costUsd: 0, kind })
    const path = join(dir, 'companies', 'teeny', 'usage.jsonl')
    await writeFile(path, [sample(1_000, 'scout', 100, 10), sample(2_000, 'scribe', 50, 5, 'planningCall')].join('\n') + '\n')
    const ledger = new MemoryLedger()
    const brain = new BrainReader({ dir })
    expect(await brain.foldUsage(ledger)).toBe(2)
    // The same file again, plus one new line: the two old ones are offered
    // again at most and counted zero more times.
    await writeFile(path, [sample(1_000, 'scout', 100, 10), sample(2_000, 'scribe', 50, 5, 'planningCall'), sample(3_000, 'scout', 1, 1)].join('\n') + '\n')
    await brain.foldUsage(ledger)

    const totals = await ledger.totals()
    expect(totals.lanes.brain).toMatchObject({ calls: 3, tokens: { input: 151, output: 16, total: 167 } })
    expect(totals.lanes.mascot.calls).toBe(0)
    expect(totals.sources['brain/scout']?.calls).toBe(2)
    expect(totals.sources['brain/scribe']?.calls).toBe(1)
    const [newest] = await ledger.recent(1)
    expect(newest).toMatchObject({ lane: 'brain', source: 'scout', model: 'flash', at: '1970-01-01T00:00:03.000Z' })
    expect((await ledger.recent(10, { lane: 'brain' })).find((r) => r.source === 'scribe')?.kind).toBe('planningCall')
  })

  it('is empty, not broken, when there is no brain and no record', async () => {
    const empty = await mkdtemp(join(tmpdir(), 'nobrain-'))
    const brain = new BrainReader({ dir: empty })
    expect(await brain.exists()).toBe(false)
    expect(await brain.foldUsage(new MemoryLedger())).toBe(0)
    const snapshot = await brain.snapshot()
    expect(snapshot.agents).toEqual([])
    expect(snapshot.runs).toEqual([])
    expect(await tailLines(join(empty, 'missing.jsonl'))).toEqual([])
  })

  it('reads only the tail of a long file, whole lines only', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'tail-'))
    const path = join(dir, 'big.jsonl')
    const lines = Array.from({ length: 1_000 }, (_, i) => `{"n":${i}}`)
    await writeFile(path, lines.join('\n') + '\n')
    const tail = await tailLines(path, 100)
    expect(tail.length).toBeGreaterThan(0)
    expect(tail.length).toBeLessThan(20)
    for (const line of tail) expect(() => JSON.parse(line)).not.toThrow()
    expect(tail[tail.length - 1]).toBe('{"n":999}')
  })
})

describe('GET /ledger and GET /brain', () => {
  const memory = { captureActivity: vi.fn(), captureMention: vi.fn(), recall: vi.fn(), captureReply: vi.fn() }

  it('serves the rows, redacted, with the totals; and the totals ride along on /mind', async () => {
    const ledger = new MemoryLedger()
    await ledger.record({ lane: 'mascot', source: 'reply', model: 'm', tokens: tokens(10, 2), input: [{ role: 'user', text: 'hi @fren' }], output: 'hey @fren' })
    await ledger.record({ lane: 'brain', source: 'scout', model: 'm', tokens: tokens(5, 1) })
    const app = buildServer({
      memory,
      consumerSecret: 'secret',
      ledger,
      mind: {
        journal: { record: vi.fn(), recent: vi.fn().mockResolvedValue([]) },
        status: { mode: 'langgraph', model: 'm', dryRun: true },
      },
    })
    const all = (await app.inject({ method: 'GET', url: '/ledger' })).json()
    expect(all.totals.lanes.mascot.tokens.total).toBe(12)
    expect(all.totals.lanes.brain.tokens.total).toBe(6)
    expect(all.entries).toHaveLength(2)
    expect(JSON.stringify(all)).not.toContain('@fren')

    const brainOnly = (await app.inject({ method: 'GET', url: '/ledger?lane=brain&limit=1' })).json()
    expect(brainOnly.entries.map((row: { source: string }) => row.source)).toEqual(['scout'])

    const mind = (await app.inject({ method: 'GET', url: '/mind' })).json()
    expect(mind.totals.lanes.mascot.calls).toBe(1)
    await app.close()
  })

  it('is 503 without a ledger or a brain', async () => {
    const app = buildServer({ memory, consumerSecret: 'secret' })
    expect((await app.inject({ method: 'GET', url: '/ledger' })).statusCode).toBe(503)
    expect((await app.inject({ method: 'GET', url: '/brain' })).statusCode).toBe(503)
    await app.close()
  })

  it('serves the brain snapshot', async () => {
    const snapshot = { source: 'live', generatedAt: 'now', company: { name: 'Teeny', slug: 'teeny' }, agents: [], workflows: [], runs: [], steps: [] }
    const brain = { snapshot: vi.fn().mockResolvedValue(snapshot) }
    const app = buildServer({ memory, consumerSecret: 'secret', brain: brain as never })
    const response = await app.inject({ method: 'GET', url: '/brain?runs=5' })
    expect(response.json()).toEqual(snapshot)
    expect(brain.snapshot).toHaveBeenCalledWith({ runs: 5, steps: 300 })
    await app.close()
  })
})

function tokens(input: number, output: number) {
  return { input, output, reasoning: 0, cached: 0, total: input + output }
}
