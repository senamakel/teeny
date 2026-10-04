import { describe, expect, it } from 'vitest'
import { cutAt } from '../src/memory/text.js'

describe('cutAt', () => {
  it('never leaves half an emoji behind', () => {
    // "ab" then an emoji: two UTF-16 units. A cut at 3 lands between them.
    const cut = cutAt('ab😀cd', 3)
    expect(cut).toBe('ab')
    expect(() => encodeURIComponent(cut)).not.toThrow()
  })

  it('keeps a whole emoji when the cut falls after it, and leaves short text alone', () => {
    expect(cutAt('ab😀cd', 4)).toBe('ab😀')
    expect(cutAt('short', 10)).toBe('short')
  })
})
