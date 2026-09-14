/**
 * The lexical screen: does this text contain a word the account does not
 * touch? Deterministic, synchronous, no model. It runs on everything coming
 * in and everything going out, and it is the part of the filter that cannot
 * be argued with, because it is not listening.
 */
import { IDIOMS, LEXICON, type ProhibitedCategory } from './lexicon.js'

export type Direction = 'inbound' | 'outbound'

export interface ScreenMatch {
  category: ProhibitedCategory
  /** The lexicon term that matched, for the log. Never the text around it. */
  term: string
}

/**
 * Leetspeak, folded. Applied as a second reading of the text rather than the
 * only one, because it turns "1488" and "ak47" into letters the digit-bearing
 * terms then cannot see; each term is tried against both readings.
 */
const LEET: Record<string, string> = {
  '0': 'o',
  '1': 'i',
  '3': 'e',
  '4': 'a',
  '5': 's',
  '7': 't',
  '@': 'a',
  $: 's',
  '!': 'i',
}

const ZERO_WIDTH = /[​-‍⁠﻿]/g

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}

/**
 * One term as a pattern: each letter may repeat ("fuuuck"), a space or hyphen
 * in the term matches any run of spacing or punctuation, and the whole thing
 * must stand on its own rather than sit inside a longer word.
 */
function termPattern(term: string, options: { looseGaps?: boolean } = {}): RegExp {
  let body = ''
  let previous = ''
  for (const char of term.toLowerCase()) {
    if (char === ' ' || char === '-') {
      // An idiom survives one stray word ("got absolutely murdered"); a term
      // does not, because a term with a gap in it is a different term.
      if (previous !== ' ') body += options.looseGaps ? '[\\s\\-_.*]+(?:[a-z]+[\\s\\-_.*]+)?' : '[\\s\\-_.*]+'
      previous = ' '
      continue
    }
    // Consecutive identical letters in the term collapse to one repeating
    // class, so "kill" reads k+i+l+ and still matches "kil" and "killll".
    // A doubled vowel is the exception: it is a different word from the
    // single one (shot/shoot, bob/boob), so the term keeps asking for two.
    // "a shot of espresso" and a film script's "one continuous shot" are
    // not "shoot"; "sh00t" still is, because the leet reading has two.
    if (char === previous && /[a-z]/.test(char)) {
      if (/[aeiou]/.test(char) && body.endsWith(`${char}+`)) body = `${body.slice(0, -1)}{2,}`
      continue
    }
    body += /[a-z]/.test(char) ? `${char}+` : escapeRegExp(char)
    previous = char
  }
  return new RegExp(`(?<![a-z0-9])${body}(?![a-z0-9])`, 'i')
}

const IDIOM_PATTERNS = IDIOMS.map((idiom) => termPattern(idiom, { looseGaps: true }))

const COMPILED = LEXICON.map((entry) => ({
  category: entry.category,
  outboundOnly: entry.outboundOnly === true,
  terms: entry.terms.map((term) => ({ term, pattern: termPattern(term) })),
}))

/** The text as the screen reads it: folded, unspaced, idioms removed. */
export function normaliseForScreen(
  text: string,
  options: { idioms?: boolean } = {},
): { plain: string; leet: string } {
  let plain = text.normalize('NFKC').toLowerCase().replace(ZERO_WIDTH, '')
  if (options.idioms !== false) {
    for (const idiom of IDIOM_PATTERNS) plain = plain.replace(idiom, ' ')
  }
  const leet = plain.replace(/[0134578@$!]/g, (char) => LEET[char] ?? char)
  return { plain, leet }
}

/**
 * The prohibited term in the text, or nothing.
 *
 * The longest matching term wins, so "kill yourself" is reported as
 * self-harm rather than as the violence of "kill": the log names the most
 * specific reason it has. Ties go to lexicon order.
 */
export function screenText(
  text: string,
  direction: Direction,
  options: { idioms?: boolean } = {},
): ScreenMatch | undefined {
  if (!text.trim()) return undefined
  const { plain, leet } = normaliseForScreen(text, options)
  let best: ScreenMatch | undefined
  for (const entry of COMPILED) {
    if (entry.outboundOnly && direction !== 'outbound') continue
    for (const { term, pattern } of entry.terms) {
      if (best && term.length <= best.term.length) continue
      if (pattern.test(plain) || pattern.test(leet)) {
        best = { category: entry.category, term }
      }
    }
  }
  return best
}
