import { describe, expect, it } from 'vitest'
import { isWithinOneLevel, mentionsDeliberately, personOf } from '../src/types.js'

describe('personOf', () => {
  it('keys a person on the stable numeric id, not the handle', () => {
    expect(personOf({ id: '1', text: 'x', authorId: '777', authorUsername: 'kaylo' })).toEqual({
      id: 'user:x-777',
      username: 'kaylo',
    })
  })

  it('is undefined when X did not return an author', () => {
    expect(personOf({ id: '1', text: 'x' })).toBeUndefined()
  })
})

describe('isWithinOneLevel', () => {
  const BOT = '555'

  it('accepts a post that is not a reply at all', () => {
    expect(isWithinOneLevel({ id: '1', text: '@bot hi' }, BOT)).toBe(true)
  })

  it('accepts a direct reply to the account', () => {
    expect(isWithinOneLevel({ id: '1', text: 'yes', inReplyToUserId: BOT }, BOT)).toBe(true)
  })

  it('rejects a reply to somebody else', () => {
    expect(isWithinOneLevel({ id: '1', text: 'yes', inReplyToUserId: '999' }, BOT)).toBe(false)
  })

  // This used to return true, on the reasoning that dropping a mention is
  // worse than answering a distant one. The account renaming itself made that
  // reasoning expensive: the id stopped resolving, every post became "close
  // enough", and it answered its own post and then that answer. A dropped
  // mention is recovered by the next poll; a public self-reply is not.
  it('rejects a reply when the account id is unknown, rather than answering everything', () => {
    expect(isWithinOneLevel({ id: '1', text: 'yes', inReplyToUserId: '999' })).toBe(false)
  })
})

describe('mentionsDeliberately', () => {
  it('sees a handle typed in the body', () => {
    expect(
      mentionsDeliberately({ id: '1', text: '@senamakel Oh hi fren @TinyMascot' }, 'TinyMascot'),
    ).toBe(true)
  })

  it('ignores the leading run X prefixes onto a reply', () => {
    expect(
      mentionsDeliberately(
        { id: '1', text: '@iamgaurangdesai @TinyMascot wen ser' },
        'TinyMascot',
      ),
    ).toBe(false)
  })

  it('sees a plain top-level mention', () => {
    expect(mentionsDeliberately({ id: '1', text: '@TinyMascot gm' }, 'TinyMascot')).toBe(false)
  })

  it('does not match a longer handle that starts the same way', () => {
    expect(
      mentionsDeliberately({ id: '1', text: 'hi @TinyMascotFan' }, 'TinyMascot'),
    ).toBe(false)
  })
})
