import { describe, expect, it, vi } from 'vitest'
import { buildServer } from '../src/server.js'
import type { Agent, Memory } from '../src/types.js'

const TOKEN = 'a-token-long-enough-to-pass'

function memory(): Memory {
  return {
    captureActivity: vi.fn().mockResolvedValue(undefined),
    captureMention: vi.fn().mockResolvedValue(undefined),
    recall: vi.fn().mockResolvedValue(''),
    captureReply: vi.fn().mockResolvedValue(undefined),
  }
}

function agent(): Agent {
  return {
    run: vi.fn().mockResolvedValue({
      output: 'done',
      steps: [{ tool: 'recall_memory', input: { query: 'x' } }],
    }),
  }
}

describe('POST /agent/run', () => {
  it('rejects requests without the bearer token', async () => {
    const app = buildServer({
      memory: memory(),
      consumerSecret: 'secret',
      agent: agent(),
      agentToken: TOKEN,
    })
    const response = await app.inject({
      method: 'POST',
      url: '/agent/run',
      payload: { task: 'say hi' },
    })

    expect(response.statusCode).toBe(401)
    await app.close()
  })

  it('runs the agent for an authorized task', async () => {
    const subject = agent()
    const app = buildServer({
      memory: memory(),
      consumerSecret: 'secret',
      agent: subject,
      agentToken: TOKEN,
    })
    const response = await app.inject({
      method: 'POST',
      url: '/agent/run',
      headers: { authorization: `Bearer ${TOKEN}` },
      payload: { task: 'summarise mentions', threadId: 'thread-1' },
    })

    expect(response.statusCode).toBe(200)
    expect(response.json()).toEqual({
      output: 'done',
      steps: [{ tool: 'recall_memory', input: { query: 'x' } }],
    })
    expect(subject.run).toHaveBeenCalledWith({
      task: 'summarise mentions',
      threadId: 'thread-1',
    })
    await app.close()
  })

  it('reports 503 when no agent is configured', async () => {
    const app = buildServer({ memory: memory(), consumerSecret: 'secret' })
    const response = await app.inject({
      method: 'POST',
      url: '/agent/run',
      headers: { authorization: `Bearer ${TOKEN}` },
      payload: { task: 'say hi' },
    })

    expect(response.statusCode).toBe(503)
    await app.close()
  })

  it('rejects a malformed task', async () => {
    const app = buildServer({
      memory: memory(),
      consumerSecret: 'secret',
      agent: agent(),
      agentToken: TOKEN,
    })
    const response = await app.inject({
      method: 'POST',
      url: '/agent/run',
      headers: { authorization: `Bearer ${TOKEN}` },
      payload: { task: '' },
    })

    expect(response.statusCode).toBe(400)
    await app.close()
  })
})
