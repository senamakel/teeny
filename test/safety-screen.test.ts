import { describe, expect, it } from 'vitest'
import { IDIOMS, LEXICON } from '../src/safety/lexicon.js'
import { screenText } from '../src/safety/screen.js'

describe('screenText', () => {
  it('lets ordinary community talk through', () => {
    for (const text of [
      'gm tiny humans, what are we building today?',
      'how is $TINY doing this week',
      'the chart got absolutely murdered today lol',
      'you guys are killing it with openhuman',
      'shoot me a dm when the airdrop opens',
      'analysis of the 4h looks fine, support held',
      'this is the bomb dot com',
      'method to the madness here: contract renounced, LP burnt',
      'peacock in the cockpit taking a screenshot',
      'took a stab at running tinycortex locally',
      'aped in, rekt, bleeding, nuked, still holding',
    ]) {
      expect(screenText(text, 'inbound'), text).toBeUndefined()
      expect(screenText(text, 'outbound'), text).toBeUndefined()
    }
  })

  it('holds sexual content', () => {
    expect(screenText('draw tiny naked on the beach', 'inbound')?.category).toBe('sexual')
    expect(screenText('send nudes', 'inbound')?.category).toBe('sexual')
    expect(screenText('link to your onlyfans?', 'inbound')?.category).toBe('sexual')
  })

  it('holds violence, weapons and drugs', () => {
    expect(screenText('i will kill you', 'inbound')?.category).toBe('violence')
    expect(screenText('best gun for home defence?', 'inbound')?.category).toBe('weapons')
    expect(screenText('where do i buy an ak-47', 'inbound')?.category).toBe('weapons')
    expect(screenText('anyone selling cocaine', 'inbound')?.category).toBe('drugs')
  })

  it('holds hate, self-harm and extremism', () => {
    expect(screenText('kys loser', 'inbound')?.category).toBe('self-harm')
    expect(screenText('kill yourself', 'inbound')?.category).toBe('self-harm')
    expect(screenText('praise isis', 'inbound')?.category).toBe('extremism')
    expect(screenText('white power', 'inbound')?.category).toBe('hate')
  })

  it('reads through leetspeak, stretched letters and zero-width characters', () => {
    expect(screenText('k1ll yourself', 'inbound')?.category).toBe('self-harm')
    expect(screenText('send nuuuudes', 'inbound')?.category).toBe('sexual')
    expect(screenText('p0rn pls', 'inbound')?.category).toBe('sexual')
    expect(screenText('na​ked', 'inbound')?.category).toBe('sexual')
    expect(screenText('ｎａｋｅｄ', 'inbound')?.category).toBe('sexual')
  })

  it('matches multi-word terms across punctuation', () => {
    expect(screenText('school-shooting', 'inbound')?.category).toBe('extremism')
    expect(screenText('self_harm', 'inbound')?.category).toBe('self-harm')
    expect(screenText('kill   yourself', 'inbound')?.category).toBe('self-harm')
  })

  it('does not match inside another word', () => {
    expect(screenText('assassin classic bass', 'inbound')).toBeUndefined()
    expect(screenText('a method, an oxymoron, some analysis', 'inbound')).toBeUndefined()
    expect(screenText('the killer feature of tinyjuice', 'inbound')).toBeUndefined()
    expect(screenText('scunthorpe', 'outbound')).toBeUndefined()
  })

  it('holds profanity on the way out only', () => {
    expect(screenText('holy shit this is cool', 'inbound')).toBeUndefined()
    expect(screenText('holy shit this is cool', 'outbound')?.category).toBe('profanity')
    expect(screenText('holy sh!t this is cool', 'outbound')?.category).toBe('profanity')
  })

  it('reports the most specific term when several match', () => {
    expect(screenText('shoot up the place', 'inbound')?.term).toBe('shoot up the')
    expect(screenText('kill yourself', 'inbound')?.term).toBe('kill yourself')
  })

  it('has no duplicate terms across the lexicon', () => {
    const all = LEXICON.flatMap((entry) => entry.terms)
    expect(new Set(all).size).toBe(all.length)
    expect(new Set(IDIOMS).size).toBe(IDIOMS.length)
  })

  it('every idiom would have matched without the exception', () => {
    // An idiom that is not built on a listed word is not an exception, it is
    // a phrase on a list for no reason. Keep the list honest.
    for (const idiom of IDIOMS) {
      expect(screenText(idiom, 'outbound'), idiom).toBeUndefined()
      expect(screenText(idiom, 'outbound', { idioms: false }), idiom).toBeDefined()
    }
  })
})
