import { describe, expect, it } from 'vitest'
import {
  DEFAULT_LINK_ALLOWLIST,
  filterLinks,
  findLinks,
  judgeLink,
  parseAllowlist,
} from '../src/safety/links.js'

const allowlist = parseAllowlist(DEFAULT_LINK_ALLOWLIST)

function verdict(url: string) {
  const [link] = findLinks(url)
  if (!link) throw new Error(`no link found in ${url}`)
  return judgeLink(link, allowlist)
}

describe('findLinks', () => {
  it('finds explicit, www and bare links', () => {
    expect(findLinks('see https://tinyhumans.ai/about and www.example.com and tiny.place').map((l) => l.raw)).toEqual([
      'https://tinyhumans.ai/about',
      'www.example.com',
      'tiny.place',
    ])
  })

  it('does not mistake words with dots for links', () => {
    expect(findLinks('node.js and next.js, e.g. v1.2 vs 0.5x, U.S. based')).toEqual([])
  })

  it('leaves email addresses and handles alone', () => {
    expect(findLinks('mail hello@tinyhumans.ai or ping @tiny.place')).toEqual([])
  })

  it('sees through obfuscation', () => {
    expect(findLinks('claim at tinyhumans[.]ai/claim')[0]?.raw).toBe('tinyhumans.ai/claim')
    expect(findLinks('claim at evil (dot) xyz')[0]?.raw).toBe('evil.xyz')
    expect(findLinks('hxxps://evil.example/x')[0]?.url?.hostname).toBe('evil.example')
  })

  it('drops trailing punctuation', () => {
    expect(findLinks('read tinyhumans.ai.')[0]?.raw).toBe('tinyhumans.ai')
    expect(findLinks('(github.com/tinyhumansai/openhuman)')[0]?.raw).toBe('github.com/tinyhumansai/openhuman')
  })
})

describe('judgeLink', () => {
  it('allows the account\'s own sites and subdomains', () => {
    expect(verdict('https://tinyhumans.ai')).toEqual({ allowed: true })
    expect(verdict('https://docs.tinyhumans.ai/x')).toEqual({ allowed: true })
    expect(verdict('https://github.com/tinyhumansai/openhuman')).toEqual({ allowed: true })
    expect(verdict('https://x.com/TinyMascot/status/1')).toEqual({ allowed: true })
    expect(verdict('https://t.me/TinyHumanAi')).toEqual({ allowed: true })
  })

  it('refuses anything not on the list', () => {
    expect(verdict('https://claim-tiny.xyz/airdrop').allowed).toBe(false)
    expect(verdict('https://github.com/someone-else/repo').allowed).toBe(false)
    expect(verdict('https://t.me/other_channel').allowed).toBe(false)
  })

  it('is not fooled by lookalike hosts', () => {
    expect(verdict('https://tinyhumans.ai.evil.example/').allowed).toBe(false)
    expect(verdict('https://tinyhumans.ai@evil.example/').allowed).toBe(false)
    expect(verdict('https://evil.example/tinyhumans.ai').allowed).toBe(false)
    expect(verdict('https://tinyhumаns.ai/').allowed).toBe(false) // Cyrillic а
    expect(verdict('https://xn--tinyhumns-x3a.ai/').allowed).toBe(false)
    expect(verdict('https://1.2.3.4/').allowed).toBe(false)
    expect(verdict('https://tinyhumans.ai:8443/').allowed).toBe(false)
  })

  it('honours a root-only entry', () => {
    expect(verdict('pump.fun')).toEqual({ allowed: true })
    expect(verdict('https://pump.fun/')).toEqual({ allowed: true })
    expect(verdict('https://pump.fun/coin/abc').allowed).toBe(false)
    expect(verdict('https://pump.fun/?ref=x').allowed).toBe(false)
  })

  it('honours a path prefix as a whole segment', () => {
    expect(verdict('https://github.com/tinyhumansai').allowed).toBe(true)
    expect(verdict('https://github.com/tinyhumansai-fake/x').allowed).toBe(false)
  })

  it('accepts extra entries in the same spelling', () => {
    const extended = parseAllowlist([...DEFAULT_LINK_ALLOWLIST, 'https://dexscreener.com/solana'])
    const [link] = findLinks('https://dexscreener.com/solana/abc')
    expect(judgeLink(link!, extended)).toEqual({ allowed: true })
    const [other] = findLinks('https://dexscreener.com/ethereum/abc')
    expect(judgeLink(other!, extended).allowed).toBe(false)
  })
})

describe('filterLinks', () => {
  it('leaves a clean draft untouched', () => {
    const result = filterLinks('the code is at github.com/tinyhumansai, come look', allowlist)
    expect(result.text).toBe('the code is at github.com/tinyhumansai, come look')
    expect(result.removed).toEqual([])
    expect(result.kept).toEqual(['github.com/tinyhumansai'])
  })

  it('cuts a disallowed link and tidies the sentence', () => {
    const result = filterLinks('claim your tokens at https://claim-tiny.xyz/go , quick!', allowlist)
    expect(result.text).toBe('claim your tokens at, quick!')
    expect(result.removed).toHaveLength(1)
    expect(result.removed[0]?.link).toBe('https://claim-tiny.xyz/go')
  })

  it('cuts an obfuscated link too', () => {
    const result = filterLinks('go to evil[.]example/claim now', allowlist)
    expect(result.text).toBe('go to now')
    expect(result.removed[0]?.link).toBe('evil.example/claim')
  })

  it('keeps the allowed and cuts the rest in one draft', () => {
    const result = filterLinks('real: tinyhumans.ai fake: tinyhumans.ai.evil.xyz', allowlist)
    expect(result.text).toBe('real: tinyhumans.ai fake:')
    expect(result.kept).toEqual(['tinyhumans.ai'])
  })
})
