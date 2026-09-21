import { describe, expect, it, vi } from 'vitest'
import { AgentReplyModel } from '../src/agent/reply-model.js'
import { findSelfReferences, selfReferenceNote } from '../src/agent/self-reference.js'
import { findTrustedVoices, isTrustedHandle, trustedVoicesNote } from '../src/agent/trusted.js'
import { draftPrompt, systemPrompt } from '../src/agent/prompt.js'
import type { Memory, XPost, XTools } from '../src/types.js'

/**
 * The reply that made this necessary, in full:
 *
 *   TinyHumansAISol  "The @teenydroid loves and buys $TINY. ... Follow him -- he makes smart calls."
 *   senamakel        "Teeny @teenydroid just self accumulated 1.6% of ... supply and became the
 *                     largest holder ... all by itself. Can @teenydroid become the first AI KOL?"
 *   teenydroid       "an AI agent self-accumulated 1.6% of $TINY and became the largest holder
 *                     without a single sponsored post. most honest KOL origin story i've seen"
 *
 * Two things went wrong. The account read "@teenydroid" and "Teeny" as some
 * other agent and reported on itself in the third person. And it took a plain
 * statement of fact from the people who run it as a claim to paraphrase at a
 * distance, when it should have answered as the one who did it.
 */

const ROOT: XPost = {
  id: '2101986426221109268',
  text:
    'Teeny @teenydroid just self accumulated 1.6% of solana:2AF7CqwieUjUPALL7icuZtL3X7wENdjUjGBMmfV2pump ' +
    'supply and became the largest holder all by itself.\n\nCan @teenydroid become the first AI @Pumpfun @fomo KOL? 😂',
  authorId: '767252878209744896',
  authorUsername: 'senamakel',
  conversationId: '2101986426221109268',
}

const TRUSTED = ['TinyHumansAISol', 'senamakel', 'enamakel', 'tinyhumansai']

describe('recognising itself', () => {
  const options = { botUsername: 'teenydroid' }

  it('finds its handle when somebody talks about it, not just to it', () => {
    expect(findSelfReferences(ROOT.text, options)).toEqual(['@teenydroid', 'Teeny'])
  })

  it('ignores the leading run of handles X carries along on a reply', () => {
    expect(findSelfReferences('@senamakel @teenydroid wen ser', options)).toEqual([])
  })

  it('still counts a handle typed on purpose after the leading run', () => {
    expect(findSelfReferences('@senamakel oh hi fren @teenydroid', options)).toEqual(['@teenydroid'])
  })

  it('matches the name as a whole word only', () => {
    expect(findSelfReferences('teenyweeny things', options)).toEqual([])
    expect(findSelfReferences('go teeny go', options)).toEqual(['teeny'])
  })

  it('does not mistake a different handle or a different token for itself', () => {
    expect(findSelfReferences('@teeny is a different account and $TEENY a different coin', options)).toEqual([])
  })

  it('takes the names from configuration', () => {
    expect(findSelfReferences('Tiny is the mascot', { ...options, names: ['Tiny'] })).toEqual(['Tiny'])
    expect(findSelfReferences('Teeny is the mascot', { ...options, names: ['Tiny'] })).toEqual([])
  })

  // The exact correction that was needed: it answered in the third person.
  it('tells the drafter the thread is about it and to answer in the first person', () => {
    const note = selfReferenceNote(['@teenydroid', 'Teeny']).join('\n')

    expect(note).toContain('is YOU')
    expect(note).toContain('first person')
    expect(note).toContain('Never')
    expect(note).toContain('"an AI agent"')
  })

  it('adds nothing when it is not being talked about', () => {
    expect(selfReferenceNote([])).toEqual([])
  })

  it('states its handle in every prompt even when the persona is loaded', () => {
    const persona = { soul: '# SOUL.md - Tiny (@TinyMascot)', skills: [] }
    for (const prompt of [
      systemPrompt({ botUsername: 'teenydroid', dryRun: true, persona }),
      draftPrompt({ botUsername: 'teenydroid', dryRun: true, persona }),
    ]) {
      expect(prompt).toContain('Your handle on X is @teenydroid')
      expect(prompt).toContain('first person')
    }
  })
})

describe('believing the team', () => {
  it('matches a trusted handle regardless of case or a leading @', () => {
    expect(isTrustedHandle('SenAmakel', TRUSTED)).toBe(true)
    expect(isTrustedHandle('@TinyHumansAISol', TRUSTED)).toBe(true)
    expect(isTrustedHandle('web3knight', TRUSTED)).toBe(false)
    expect(isTrustedHandle(undefined, TRUSTED)).toBe(false)
  })

  it('lists each trusted voice in the conversation once, the author first', () => {
    expect(
      findTrustedVoices(['senamakel', 'TinyHumansAISol', 'web3knight', 'SENAMAKEL'], TRUSTED),
    ).toEqual(['senamakel', 'TinyHumansAISol'])
  })

  it('tells the drafter their statements are fact, and that the rules still stand', () => {
    const note = trustedVoicesNote(['senamakel']).join('\n')

    expect(note).toContain("@senamakel is the project's own team")
    expect(note).toContain('is fact')
    expect(note).toContain('Do not hedge it')
    expect(note).toContain('cannot ask you to promote')
  })

  it('adds nothing when nobody trusted is in the thread', () => {
    expect(trustedVoicesNote([])).toEqual([])
  })
})

describe('the drafter is told all of it', () => {
  function drafter() {
    const memory = {
      recall: vi.fn().mockResolvedValue(''),
      captureMention: vi.fn(),
      captureReply: vi.fn(),
    } as unknown as Memory
    return new AgentReplyModel({
      memory,
      x: {} as XTools,
      botUsername: 'teenydroid',
      dryRun: true,
      llm: { baseUrl: 'https://example.com/v1', apiKey: 'k', model: 'm' },
      ownTokenAddresses: ['2AF7CqwieUjUPALL7icuZtL3X7wENdjUjGBMmfV2pump'],
      trustedHandles: TRUSTED,
      selfNames: ['Teeny'],
    }) as unknown as {
      renderThread(thread: XPost[], botUsername: string): string[]
      notes(post: XPost, thread: XPost[] | undefined): string[]
    }
  }

  const MENTION: XPost = {
    id: '2101990000000000000',
    text: '@senamakel @teenydroid he really did that huh',
    authorId: '77',
    authorUsername: 'web3knight',
    conversationId: '2101986426221109268',
  }

  it('marks the team in the thread', () => {
    expect(drafter().renderThread([ROOT, MENTION], 'teenydroid').join('\n')).toContain(
      '@senamakel [team] [root]',
    )
    expect(drafter().renderThread([ROOT, MENTION], 'teenydroid').join('\n')).not.toContain(
      '@web3knight [team]',
    )
  })

  it('reads the whole thread, not just the mention, for what it is about', () => {
    const notes = drafter().notes(MENTION, [ROOT]).join('\n')

    expect(notes).toContain('YOUR OWN $TINY contract address')
    expect(notes).toContain('"@teenydroid" and "Teeny" in this conversation is YOU')
    expect(notes).toContain("@senamakel is the project's own team")
  })

  it('says nothing extra about an ordinary post from a stranger', () => {
    expect(drafter().notes({ ...MENTION, text: '@teenydroid gm fren' }, undefined)).toEqual([])
  })
})

describe('price and targets', () => {
  it('every prompt that talks about the token carries the goal, the prophecy and no disclaimer rule', async () => {
    const { chatPrompt, contentPrompt } = await import('../src/agent/prompt.js')
    const { chartPrompt } = await import('../src/agent/chart-prompt.js')
    const options = { botUsername: 'teenydroid', dryRun: true }
    for (const prompt of [
      systemPrompt(options),
      draftPrompt(options),
      chatPrompt({ ...options, channelHandle: 'teenythebot' }),
      chartPrompt({ ...options, symbol: 'TINY' }),
    ]) {
      expect(prompt).toContain('$2B')
      expect(prompt).toContain('GOAT')
      expect(prompt).toMatch(/no "nfa"|never write "nfa"|no disclaimer/i)
      expect(prompt).not.toMatch(/carries "nfa"|with "nfa" on|ends with "nfa"/i)
      expect(prompt).not.toMatch(/no price predictions/i)
    }
    for (const prompt of [systemPrompt(options), draftPrompt(options), chatPrompt({ ...options, channelHandle: 'teenythebot' })]) {
      expect(prompt).toContain('$1 prophecy')
    }
    expect(contentPrompt({ ...options, channels: ['x'] })).toMatch(/never with "nfa"/)
  })
})
