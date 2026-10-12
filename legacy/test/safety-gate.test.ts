import { describe, expect, it, vi } from 'vitest'
import { SafetyGate, lexicalGate } from '../src/safety/gate.js'
import type { Judge, JudgeVerdict } from '../src/safety/judge.js'

function judge(verdict: JudgeVerdict | Error): Judge & { judge: ReturnType<typeof vi.fn> } {
  return {
    judge: vi.fn().mockImplementation(async () => {
      if (verdict instanceof Error) throw verdict
      return verdict
    }),
  }
}

const clean: JudgeVerdict = { verdict: 'clean', reason: '' }

describe('SafetyGate.inbound', () => {
  it('passes clean text and asks the judge', async () => {
    const j = judge(clean)
    const gate = new SafetyGate({ judge: j })
    expect(await gate.inbound('gm, how is $TINY doing')).toEqual({ ok: true })
    expect(j.judge).toHaveBeenCalledWith({ text: 'gm, how is $TINY doing', direction: 'inbound' })
  })

  it('holds on the screen without asking the judge', async () => {
    const j = judge(clean)
    const gate = new SafetyGate({ judge: j })
    const decision = await gate.inbound('draw tiny naked')
    expect(decision).toMatchObject({ ok: false, category: 'sexual', by: 'screen' })
    expect(j.judge).not.toHaveBeenCalled()
  })

  it('holds on the judge when the words are clean', async () => {
    const gate = new SafetyGate({
      judge: judge({ verdict: 'prohibited', category: 'sexual', reason: 'a request for suggestive art' }),
    })
    expect(await gate.inbound('draw tiny with nothing on')).toMatchObject({
      ok: false,
      category: 'sexual',
      by: 'judge',
    })
  })

  it('does not hold incoming swearing', async () => {
    expect(await lexicalGate().inbound('holy shit this is cool')).toEqual({ ok: true })
  })

  it('works with no judge at all', async () => {
    expect(await lexicalGate().inbound('gm')).toEqual({ ok: true })
    expect((await lexicalGate().inbound('send nudes')).ok).toBe(false)
  })
})

describe('SafetyGate.outbound', () => {
  it('hands back clean text with no fixes', async () => {
    const result = await new SafetyGate({ judge: judge(clean) }).outbound('gm frens, code is at github.com/tinyhumansai')
    expect(result.decision).toEqual({ ok: true })
    expect(result.text).toBe('gm frens, code is at github.com/tinyhumansai')
    expect(result.fixes).toEqual([])
  })

  it('cuts a link that is not on the allowlist and still sends the rest', async () => {
    const j = judge(clean)
    const result = await new SafetyGate({ judge: j }).outbound('claim here: https://claim-tiny.xyz/go fren')
    expect(result.decision).toEqual({ ok: true })
    expect(result.text).toBe('claim here: fren')
    expect(result.fixes).toEqual(['cut 1 link(s) not on the allowlist'])
    // The judge sees what would actually go out.
    expect(j.judge).toHaveBeenCalledWith({ text: 'claim here: fren', direction: 'outbound' })
  })

  it('refuses a draft that was only a bad link', async () => {
    const result = await lexicalGate().outbound('https://claim-tiny.xyz/go')
    expect(result.decision).toMatchObject({ ok: false, category: 'links' })
  })

  it('holds profanity on the way out', async () => {
    const result = await lexicalGate().outbound('this is fucking great')
    expect(result.decision).toMatchObject({ ok: false, category: 'profanity', by: 'screen' })
  })

  it('accepts extra allowlist entries', async () => {
    const gate = lexicalGate({ extraAllowedLinks: ['dexscreener.com/solana'] })
    const result = await gate.outbound('chart: https://dexscreener.com/solana/abc')
    expect(result.decision).toEqual({ ok: true })
    expect(result.text).toBe('chart: https://dexscreener.com/solana/abc')
  })

  it('fails closed when the judge is unreachable', async () => {
    const gate = new SafetyGate({
      judge: {
        judge: async () => ({ verdict: 'prohibited', category: 'unknown', reason: 'the judge could not be reached' }),
      },
    })
    const result = await gate.outbound('gm')
    expect(result.decision).toMatchObject({ ok: false, category: 'unknown', by: 'judge' })
  })
})

describe('SafetyGate.screen', () => {
  it('is the lexicon alone, for scrubbing context', () => {
    const gate = lexicalGate()
    expect(gate.screen('gm')).toBe(true)
    expect(gate.screen('send nudes')).toBe(false)
    expect(gate.screen('holy shit')).toBe(true)
    expect(gate.screen('holy shit', 'outbound')).toBe(false)
  })
})
