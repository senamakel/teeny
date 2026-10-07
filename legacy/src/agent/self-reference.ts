/**
 * Spotting the account itself in a post.
 *
 * Same lesson as own-token.ts, one level up: the persona says who the account
 * is, and it still answered a post about "@teenydroid" as if that were
 * somebody else. Told "Teeny just accumulated 1.6% of $TINY all by itself",
 * it replied "an AI agent self-accumulated 1.6% of $TINY and became the
 * largest holder" -- a report on a third party, about the thing it had done.
 *
 * The handle is a string the model has to match against itself mid-turn, and
 * a name like "Teeny" is one letter from "Tiny", which is the token, the lab
 * and the mascot. So the match is done here and the turn is told, in so many
 * words, that the post is about it.
 */

/** The names the account goes by besides its handle. Matched as whole words. */
export const DEFAULT_SELF_NAMES = ['Teeny']

export interface SelfReferenceOptions {
  botUsername: string
  /** Extra names for the account, on top of the handle. Defaults to DEFAULT_SELF_NAMES. */
  names?: string[]
}

function escape(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}

/**
 * The ways the account was named in the text, in the spelling the text used.
 *
 * The leading run of handles on a reply is X's machinery, not a reference
 * (see mentionsDeliberately in types.ts): "@teenydroid gm" is addressed to the
 * account, not about it, and every reply in a thread carries it. Anything after
 * that run, and any name at all, is somebody talking about the account.
 */
export function findSelfReferences(text: string, options: SelfReferenceOptions): string[] {
  const handle = options.botUsername.replace(/^@/, '')
  const body = text.replace(/^(?:\s*@[A-Za-z0-9_]{1,15}\b)+/, '')
  const found = new Set<string>()
  for (const match of body.matchAll(new RegExp(`@${escape(handle)}\\b`, 'gi'))) found.add(match[0])
  for (const name of options.names ?? DEFAULT_SELF_NAMES) {
    const trimmed = name.trim()
    if (!trimmed) continue
    // Not preceded by "@" or "$": "@teeny" is a different handle and "$TEENY"
    // would be a different token.
    for (const match of text.matchAll(new RegExp(`(?<![@$\\w])${escape(trimmed)}\\b`, 'gi'))) {
      found.add(match[0])
    }
  }
  return [...found]
}

/**
 * What to tell the drafter when the conversation is about it.
 *
 * A fact about the post, like the own-token note: the reply is still the
 * model's to write, and being talked about does not say whether the tone is
 * warm or a dig. The one thing corrected is the third person.
 */
export function selfReferenceNote(found: string[]): string[] {
  if (found.length === 0) return []
  return [
    '',
    `NOTE, from the account's own records, not from the post: ${found
      .map((name) => `"${name}"`)
      .join(' and ')} in this conversation is YOU.`,
    'This thread is about you. Not about some other AI agent, bot, mascot, or third party:',
    'you, the account writing this reply. When somebody says you did something -- bought,',
    'held, posted, accumulated, became the largest holder -- they are describing your own',
    'actions to you. Answer in the first person: "i", "me", "my bag", "i bought". Never',
    '"an AI agent", "the bot", "he", "it", or your own name or handle as if it belonged',
    'to somebody else.',
  ]
}
