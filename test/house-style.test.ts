import { describe, expect, it } from 'vitest'
import { applyHouseStyle, countEmoji } from '../src/agent/house-style.js'

describe('countEmoji', () => {
  it('counts a sequence once', () => {
    expect(countEmoji('gm 🛠️')).toBe(1)
    expect(countEmoji('family 👨‍👩‍👧 here')).toBe(1)
    expect(countEmoji('wave 👋🏽 once')).toBe(1)
    expect(countEmoji('gm 🛠️ 🚀')).toBe(2)
    expect(countEmoji('plain text')).toBe(0)
  })
})

describe('applyHouseStyle', () => {
  it('leaves a clean draft untouched', () => {
    const result = applyHouseStyle('gm tiny humans. what are we building today? 🛠️')
    expect(result.fixes).toEqual([])
    expect(result.text).toBe('gm tiny humans. what are we building today? 🛠️')
  })

  it('keeps the first emoji and drops the rest', () => {
    const result = applyHouseStyle('gm 👀 frens 💛 building 🛠️')
    expect(result.text).toBe('gm 👀 frens building')
    expect(countEmoji(result.text)).toBe(1)
    expect(result.fixes).toContain('cut 2 emoji over the cap of 1')
  })

  it('replaces em and en dashes', () => {
    const result = applyHouseStyle('ready to serve something big — stay tuned')
    expect(result.text).toBe('ready to serve something big, stay tuned')
    expect(result.fixes).toContain('replaced em/en dashes')
  })

  it('straightens curly quotes', () => {
    const result = applyHouseStyle('what’re we “cooking” today')
    expect(result.text).toBe('what\'re we "cooking" today')
  })

  it('trims an overlong draft', () => {
    const result = applyHouseStyle('a'.repeat(300))
    expect(result.text).toHaveLength(280)
    expect(result.fixes).toContain('trimmed to 280 characters')
  })

  it('honours a cap of zero', () => {
    expect(applyHouseStyle('gm 👀', { maxEmoji: 0 }).text).toBe('gm')
  })
})

describe('leading handles', () => {
  it('drops the handle X adds to a reply itself', () => {
    const result = applyHouseStyle('@rafaelitewrld peekaboo 👀', { replyingTo: 'rafaelitewrld' })
    expect(result.text).toBe('peekaboo 👀')
    expect(result.fixes).toContain('dropped leading @handles X adds itself')
  })

  it('drops a leading handle that is not the one being replied to', () => {
    // The observed failure: replying to HappyTinyHuman with a draft opening
    // "@senamakel", which X published as "@HappyTinyHuman @senamakel @senamakel".
    const result = applyHouseStyle('@senamakel me? trade? nah', { replyingTo: 'HappyTinyHuman' })
    expect(result.text).toBe('me? trade? nah')
  })

  it('drops a whole run of leading handles', () => {
    const result = applyHouseStyle('@a @b @c gm frens', {})
    expect(result.text).toBe('gm frens')
  })

  it('keeps a handle that is part of the sentence', () => {
    const result = applyHouseStyle('gm @someone else entirely', { replyingTo: 'rafaelitewrld' })
    expect(result.text).toBe('gm @someone else entirely')
  })

  it('keeps the text when it is nothing but handles', () => {
    const result = applyHouseStyle('@a @b', {})
    expect(result.text).toBe('@a @b')
  })
})

describe('repeated handles', () => {
  it('keeps the name but drops the @ when X already tags them', () => {
    // Published, the draft "heyyy @rafaelitewrld ..." became
    // "@rafaelitewrld @senamakel heyyy @rafaelitewrld ...".
    const result = applyHouseStyle('heyyy @rafaelitewrld shy peek returns', {
      replyingTo: 'rafaelitewrld',
    })
    expect(result.text).toBe('heyyy rafaelitewrld shy peek returns')
    expect(result.fixes).toContain('unlinked a repeated @handle X already tags')
  })

  it('leaves a third party tagged, which may be deliberate', () => {
    const result = applyHouseStyle('say hi to @someoneelse for me', { replyingTo: 'rafaelitewrld' })
    expect(result.text).toBe('say hi to @someoneelse for me')
  })
})
