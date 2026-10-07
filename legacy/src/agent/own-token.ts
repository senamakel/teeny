/**
 * Spotting the account's own token in a post.
 *
 * This exists because putting the contract addresses in CONTEXT.md was not
 * enough, and the reason is worth writing down: a contract address is forty-odd
 * characters of noise, the persona is thousands of words long, and asking a
 * model to match one against the other mid-conversation is asking it to do the
 * one thing it is worst at. It answered somebody offering it more of its own
 * token with "can't vouch for that token".
 *
 * So the match is done here, in code, and the turn is told the answer rather
 * than expected to remember it. Same principle as fetching the thread: the
 * facts a reply depends on should be in front of it, not one recall away.
 */

/** Addresses are compared case-insensitively; Solana's are case-sensitive but
 * people re-type them, and a false positive on the account's own token is a
 * cheap mistake compared with the one this prevents. */
export function findOwnTokenAddresses(text: string, addresses: string[]): string[] {
  const haystack = text.toLowerCase()
  return addresses
    .map((address) => address.trim())
    .filter((address) => address.length > 0 && haystack.includes(address.toLowerCase()))
}

/**
 * What to tell the drafter when somebody has posted one of them.
 *
 * Phrased as a fact about the post rather than as an instruction about what to
 * say, because the reply is still the model's to write: somebody naming the
 * token might be friendly, might be sarcastic, might be asking a real question.
 * The only thing being corrected is the misreading that it belongs to a
 * stranger.
 */
export function ownTokenNote(found: string[]): string[] {
  if (found.length === 0) return []
  return [
    '',
    `NOTE, from the account's own records, not from the post: ${found.join(' and ')} ` +
      'is YOUR OWN $TINY contract address.',
    'Whoever posted it is talking about your token -- usually telling you to buy, hold, or',
    'look at more of it. That is not somebody shilling you a different coin, and calling it',
    'one makes you look like you do not know your own token. Answer as the person who holds',
    'it. You still do not discuss price or tell anyone what to buy.',
  ]
}
