/**
 * What may be sent, decided outside the model.
 *
 * The reply path's entire input is text written by strangers, so the airdrop
 * tool is a wallet a stranger can address. A prompt rule is an argument and an
 * argument can be won; every limit that matters is therefore checked here,
 * against the ledger and the config, after the model has already asked.
 *
 * The shape of the defence is to make the worst case boring. An attacker who
 * writes the most persuasive post in the world gets: their own address, once,
 * for at most one call's cap, and only while the day's budget lasts. That is
 * not an exploit, it is an airdrop working as intended.
 */

export interface AirdropRequest {
  /** Where the tokens would go. */
  to: string
  /** How much the model asked for, in whole tokens. */
  amount: number
  /** The X user the reply is addressed to, keyed by numeric id, not handle. */
  recipientUserId?: string
  /** The post being answered, and its text, for the address-in-post rule. */
  post?: { id: string; text: string; authorId?: string }
  /**
   * Whether they follow the account. `undefined` means X could not be asked,
   * which is not the same as "no" and is not treated as one.
   */
  isFollower?: boolean
}

export interface AirdropLimits {
  /** The most one call may send. The model cannot raise this by asking. */
  maxPerAirdrop: number
  /** The most that may leave the wallet in a rolling 24 hours. */
  maxPerDay: number
  /** How many times one X account may ever be paid. */
  maxPerRecipient: number
  /**
   * Whether the destination must appear in the post being answered.
   *
   * With it on, an address is only payable by the person who published it, in
   * the post the agent is answering, which bounds an injected address to the
   * account that injected it. With it off, any address anywhere in the turn --
   * a search result, a recalled memory, an older post in the thread -- is a
   * destination, and none of those were written by the person being answered.
   */
  requireAddressInPost: boolean
  /**
   * Whether the recipient has to follow the account.
   *
   * It is a weak signal -- a follow costs nothing and a farm can buy a thousand
   * -- so it is a filter for drive-by begging rather than a security control.
   * The caps are what make the worst case survivable; this only makes the
   * common case quieter.
   */
  requireFollow: boolean
}

export interface LedgerSnapshot {
  /** Whole tokens already sent in the trailing 24 hours. */
  sentToday: number
  /** How many times this X account has been paid before. */
  paidRecipientCount: number
  /** A prior airdrop for this exact post, if the tool is called twice. */
  existingTxHash?: string
  /**
   * The X id already paid at this address, when it is somebody else.
   *
   * Twenty new accounts pointing at one wallet is the ordinary shape of airdrop
   * farming, and asking only "has this account been paid" cannot see it.
   */
  addressOwnedBy?: string
}

export type AirdropDecision =
  | { allowed: true; amount: number }
  | { allowed: false; reason: string }

/** Addresses are 20 bytes of hex. Anything else is not a destination. */
const ADDRESS = /^0x[0-9a-fA-F]{40}$/

/**
 * The zero address and the conventional burn address are valid hex and the
 * chain would accept both. Sending there is indistinguishable from setting the
 * tokens on fire, and it is what a confused model reaches for when the post it
 * is answering contains no real address.
 */
const UNSPENDABLE = new Set([
  '0x0000000000000000000000000000000000000000',
  '0x000000000000000000000000000000000000dead',
])

export function decideAirdrop(
  request: AirdropRequest,
  limits: AirdropLimits,
  ledger: LedgerSnapshot,
): AirdropDecision {
  const to = request.to?.trim() ?? ''

  if (!ADDRESS.test(to)) {
    return {
      allowed: false,
      reason: 'that is not a wallet address: expected 0x and 40 hex characters',
    }
  }
  if (UNSPENDABLE.has(to.toLowerCase())) {
    return { allowed: false, reason: 'that address burns the tokens rather than delivering them' }
  }

  if (!Number.isFinite(request.amount) || request.amount <= 0) {
    return { allowed: false, reason: 'the amount must be a positive number of tokens' }
  }
  // Fractions are a rounding argument nobody needs to have on a giveaway, and
  // they make every total in the ledger harder to read.
  if (!Number.isInteger(request.amount)) {
    return { allowed: false, reason: 'the amount must be a whole number of tokens' }
  }
  if (request.amount > limits.maxPerAirdrop) {
    return {
      allowed: false,
      reason: `${request.amount} is over the ${limits.maxPerAirdrop} cap for a single airdrop`,
    }
  }

  if (ledger.existingTxHash) {
    return { allowed: false, reason: `this post was already paid: ${ledger.existingTxHash}` }
  }

  if (ledger.paidRecipientCount >= limits.maxPerRecipient) {
    return {
      allowed: false,
      reason:
        limits.maxPerRecipient === 1
          ? 'this account has had its airdrop already'
          : `this account has had ${ledger.paidRecipientCount} airdrops, which is the limit`,
    }
  }

  if (ledger.sentToday + request.amount > limits.maxPerDay) {
    const left = Math.max(0, limits.maxPerDay - ledger.sentToday)
    return {
      allowed: false,
      reason: left > 0 ? `only ${left} left in today's budget` : "today's airdrop budget is spent",
    }
  }

  if (
    ledger.addressOwnedBy &&
    (!request.recipientUserId || ledger.addressOwnedBy !== request.recipientUserId)
  ) {
    return { allowed: false, reason: 'that wallet has already been paid, under another account' }
  }

  if (limits.requireFollow) {
    if (request.isFollower === undefined) {
      // Money, so an unanswerable question is a no. Not sending is recovered
      // by them asking again; sending wrongly is not recovered at all.
      return { allowed: false, reason: 'could not check whether they follow the account' }
    }
    if (!request.isFollower) {
      return { allowed: false, reason: 'they do not follow the account' }
    }
  }

  if (limits.requireAddressInPost) {
    const post = request.post
    if (!post) {
      return { allowed: false, reason: 'no post to check the address against' }
    }
    // Case-insensitive: people paste addresses in every capitalisation, and a
    // checksummed address differs from what a phone keyboard produces.
    if (!post.text.toLowerCase().includes(to.toLowerCase())) {
      return {
        allowed: false,
        reason: 'that address is not in the post being answered, so it is not theirs to claim',
      }
    }
    // Whoever wrote the post is who gets paid. Without this, an attacker can
    // publish an address, be refused, and have somebody else's later reply in
    // the same thread pay it.
    if (request.recipientUserId && post.authorId && request.recipientUserId !== post.authorId) {
      return { allowed: false, reason: 'the address was published by somebody else in this thread' }
    }
  }

  return { allowed: true, amount: request.amount }
}
