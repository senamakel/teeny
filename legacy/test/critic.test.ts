import { describe, expect, it, vi } from 'vitest'
import { createCritiqueTools } from '../src/agent/tools/critique.js'
import {
  ACTIONS,
  RISKY_COST,
  WEIGHTS,
  WORTHY_SCORE,
  renderCritique,
  scoreActions,
  toCritique,
  verdictOf,
  type Action,
  type Critic,
  type Critique,
  type CritiqueAnswer,
} from '../src/critic/algo.js'
import { buildServer } from '../src/server.js'
import type { Memory } from '../src/types.js'

const zero = Object.fromEntries(ACTIONS.map((action) => [action, 0])) as Record<Action, number>
const predicted = (some: Partial<Record<Action, number>>): Record<Action, number> => ({ ...zero, ...some })

function answer(overrides: Partial<CritiqueAnswer> = {}): CritiqueAnswer {
  return {
    ...zero,
    communityReply: 0,
    firstReply: '',
    disagreement: '',
    shareReason: '',
    riskReason: '',
    fixes: [],
    summary: 'a read',
    ...overrides,
  }
}

describe('the ranker arithmetic', () => {
  it('is the param.rs weighted sum: a reply is ten likes, a copied link forty, one report erases forty-seven replies', () => {
    expect(WEIGHTS.reply / WEIGHTS.like).toBe(10)
    expect(WEIGHTS.shareViaCopyLink / WEIGHTS.like).toBe(40)
    expect(Math.floor(-WEIGHTS.report / WEIGHTS.reply)).toBe(46)
    const { score, upside, cost } = scoreActions(predicted({ reply: 0.3, like: 0.5, shareViaCopyLink: 0.05, report: 0.01 }), 0.1)
    // 0.3*5 + 0.5*0.5 + 0.05*20 + 0.1*15 = 1.5 + 0.25 + 1 + 1.5 = 4.25; report 0.01*-234 = -2.34
    expect(upside).toBe(4.25)
    expect(cost).toBe(-2.34)
    expect(score).toBe(1.91)
  })

  it('calls a draft worthy over the line, flat under it, and risky when the downside is the story whatever the upside', () => {
    expect(verdictOf({ score: WORTHY_SCORE, cost: 0 })).toBe('worthy')
    expect(verdictOf({ score: WORTHY_SCORE - 0.01, cost: 0 })).toBe('flat')
    expect(verdictOf({ score: 30, cost: RISKY_COST })).toBe('risky')
    // A likes-only post scores a sixth: flat. One that a third would answer: worthy.
    expect(verdictOf(scoreActions(predicted({ like: 0.3 }), 0))).toBe('flat')
    expect(verdictOf(scoreActions(predicted({ reply: 0.3, quote: 0.05, like: 0.2 }), 0.05))).toBe('worthy')
    // One in fifty reporting sinks anything; so does one in ten tapping "not interested".
    expect(verdictOf(scoreActions(predicted({ reply: 0.5, shareViaCopyLink: 0.2, report: 0.02 }), 0.3))).toBe('risky')
    expect(verdictOf(scoreActions(predicted({ reply: 0.2, notInterested: 0.1 }), 0.1))).toBe('risky')
    expect(verdictOf(scoreActions(predicted({ reply: 0.3, notInterested: 0.05 }), 0.1))).toBe('flat')
  })

  it('turns the model answer into a critique and renders it verdict first, fixes last', () => {
    const critique = toCritique(
      answer({ reply: 0.02, like: 0.2, firstReply: 'gm', fixes: ['end on the question', 'cut the closing line'], summary: 'a greeting plus a fact' }),
    )
    expect(critique.verdict).toBe('flat')
    expect(critique.predicted.reply).toBe(0.02)
    const text = renderCritique(critique)
    expect(text.startsWith('Verdict: FLAT (ranker score 0.2')).toBe(true)
    expect(text).toContain('First reply a stranger types: "gm"')
    expect(text).toContain('1. end on the question\n2. cut the closing line')
    expect(renderCritique(toCritique(answer()))).toContain('nobody has one')
  })
})

function critic(critiques: Array<Critique | Error>): Critic {
  const queue = [...critiques]
  return {
    critique: vi.fn().mockImplementation(async () => {
      const next = queue.shift()
      if (!next) throw new Error('the critic was asked more often than expected')
      if (next instanceof Error) throw next
      return next
    }),
  }
}

const flat = toCritique(answer({ reply: 0.01, like: 0.1, fixes: ['ask what they run locally'], summary: 'flat' }))
const worthy = toCritique(answer({ reply: 0.3, quote: 0.1, like: 0.3, communityReply: 0.1, firstReply: 'mine is a for-loop', summary: 'worthy' }))

describe('the x_critique tool', () => {
  it('hands the draft, the media line and the context to the critic and returns the rendered read', async () => {
    const reader = critic([flat])
    const [tool] = createCritiqueTools(reader)
    const result = await tool!.invoke({ kind: 'caption', text: 'gm. a lab shipped a model.', media: 'Tiny at a desk', context: 'Story: a model' })
    expect(reader.critique).toHaveBeenCalledWith({ kind: 'caption', text: 'gm. a lab shipped a model.', media: 'Tiny at a desk', context: 'Story: a model' })
    expect(result).toContain('Verdict: FLAT')
    expect(result).toContain('1. ask what they run locally')
  })

  it('fails open in words when the critic is down', async () => {
    const [tool] = createCritiqueTools(critic([new Error('502')]))
    const result = await tool!.invoke({ text: 'gm' })
    expect(result).toContain('could not be reached (502)')
    expect(result).toContain('Use your own read')
  })
})

describe('POST /critique', () => {
  const memory = (): Memory => ({
    captureActivity: vi.fn(),
    captureMention: vi.fn(),
    recall: vi.fn().mockResolvedValue(''),
    captureReply: vi.fn(),
  })
  const TOKEN = 'a-token-long-enough-to-pass'

  it('is 503 without a critic, 401 without the token, 400 without text, and the critique with its rendering otherwise', async () => {
    const none = buildServer({ memory: memory(), consumerSecret: 's', agentToken: TOKEN })
    expect((await none.inject({ method: 'POST', url: '/critique', payload: { text: 'gm' } })).statusCode).toBe(503)

    const reader = critic([worthy])
    const app = buildServer({ memory: memory(), consumerSecret: 's', agentToken: TOKEN, critic: reader })
    expect((await app.inject({ method: 'POST', url: '/critique', payload: { text: 'gm' } })).statusCode).toBe(401)
    const headers = { authorization: `Bearer ${TOKEN}` }
    expect((await app.inject({ method: 'POST', url: '/critique', headers, payload: { kind: 'script' } })).statusCode).toBe(400)

    const response = await app.inject({ method: 'POST', url: '/critique', headers, payload: { kind: 'script', text: '0-2s: Tiny sips.' } })
    expect(response.statusCode).toBe(200)
    const body = response.json() as Critique & { rendered: string }
    expect(body.verdict).toBe('worthy')
    expect(body.rendered).toContain('Verdict: WORTHY')
    expect(reader.critique).toHaveBeenCalledWith({ kind: 'script', text: '0-2s: Tiny sips.' })
  })

  it('is 502 when the critic cannot be reached', async () => {
    const app = buildServer({ memory: memory(), consumerSecret: 's', agentToken: 'a-token-long-enough-to-pass', critic: critic([new Error('down')]) })
    const response = await app.inject({
      method: 'POST',
      url: '/critique',
      headers: { authorization: 'Bearer a-token-long-enough-to-pass' },
      payload: { text: 'gm' },
    })
    expect(response.statusCode).toBe(502)
  })
})
