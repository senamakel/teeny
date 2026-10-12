import { createHmac, timingSafeEqual } from 'node:crypto'
import { isWorthAnswering, type Memory, type XPost } from '../types.js'
import type { Interaction } from '../cadence/people.js'

/**
 * Two shapes arrive here, because X has two webhook products and the
 * account moved from one to the other on 2026-09-14.
 *
 * Account Activity (the old one, deprecated) delivers v1.1-shaped tweet
 * objects under `tweet_create_events`: snake_case `id_str` rather than `id`,
 * the author nested under `user`, and the text truncated into
 * `extended_tweet.full_text` once it passes 140 characters.
 *
 * The X Activity API delivers one event per request as
 * `{ data: { event_type, event_uuid, filter, tag, payload, includes } }`,
 * where a `post.*` event's payload is a v2 post and the author's handle is
 * in `includes.users`, when it is anywhere.
 */
interface ActivityTweet {
  id_str?: string
  text?: string
  retweeted_status?: unknown
  truncated?: boolean
  created_at?: string
  in_reply_to_user_id_str?: string | null
  extended_tweet?: { full_text?: string }
  user?: { id_str?: string; screen_name?: string }
}

function toPost(tweet: ActivityTweet): XPost | undefined {
  const text = tweet.extended_tweet?.full_text ?? tweet.text
  if (!tweet.id_str || !text) return undefined
  return {
    id: tweet.id_str,
    text,
    ...(tweet.user?.id_str ? { authorId: tweet.user.id_str } : {}),
    ...(tweet.user?.screen_name ? { authorUsername: tweet.user.screen_name } : {}),
    ...(tweet.created_at ? { createdAt: new Date(tweet.created_at).toISOString() } : {}),
    ...(tweet.in_reply_to_user_id_str
      ? { inReplyToUserId: tweet.in_reply_to_user_id_str }
      : {}),
  }
}

/**
 * The mentions in a delivery that are worth answering.
 *
 * The account's own posts arrive here too, because the subscription is to the
 * account rather than to other people's posts about it, and replying to itself
 * is the one loop this service must never enter.
 */
export function mentionsFromActivity(
  payload: Record<string, unknown>,
  options: { botUserId: string; botUsername: string },
): XPost[] {
  // Required, not optional. A delivery is mostly the account's own activity,
  // so without an id to compare against there is no way to tell the account's
  // posts from anybody else's, and the safe reading of every one of them is
  // "not a mention".
  if (!options.botUserId) return []

  const event = activityEvent(payload)
  if (event) {
    if (!POST_EVENTS.has(event.eventType)) return []
    const post = fromV2Post(event.payload, event.includes)
    if (!post) return []
    if (/^RT @[A-Za-z0-9_]{1,15}:/.test(post.text)) return []
    if (post.authorId === options.botUserId) return []
    if (!isWorthAnswering(post, options)) return []
    return [post]
  }

  const events = payload.tweet_create_events
  if (!Array.isArray(events)) return []

  return (events as ActivityTweet[]).flatMap((tweet) => {
    // A retweet of the account carries "RT @handle: ..." as its text, so it
    // reads as a mention of the account by someone else. Replying to one is
    // worse than useless: X attaches the reply to the original, so the account
    // answers itself, and the reply inherits every handle the original named.
    // The polling query excludes these with -is:retweet; this is that filter.
    if (tweet.retweeted_status || /^RT @[A-Za-z0-9_]{1,15}:/.test(tweet.text ?? '')) return []

    const post = toPost(tweet)
    if (!post) return []
    if (post.authorId === options.botUserId) return []
    if (!isWorthAnswering(post, options)) return []
    return [post]
  })
}

/** The X Activity API events whose payload is somebody's post about the account. */
const POST_EVENTS = new Set(['post.mention.create', 'post.reply.create', 'post.quote.create'])

interface ActivityIncludes {
  users?: Array<{ id?: string; username?: string }>
}

interface ActivityEvent {
  eventType: string
  eventId?: string
  payload: Record<string, unknown>
  includes?: ActivityIncludes
}

/** The X Activity API envelope, when the delivery is one. */
export function activityEvent(payload: Record<string, unknown>): ActivityEvent | undefined {
  const data = payload.data
  if (!data || typeof data !== 'object' || Array.isArray(data)) return undefined
  const envelope = data as Record<string, unknown>
  if (typeof envelope.event_type !== 'string') return undefined
  const body =
    envelope.payload && typeof envelope.payload === 'object' ? (envelope.payload as Record<string, unknown>) : {}
  return {
    eventType: envelope.event_type,
    ...(typeof envelope.event_uuid === 'string' ? { eventId: envelope.event_uuid } : {}),
    payload: body,
    ...(envelope.includes && typeof envelope.includes === 'object'
      ? { includes: envelope.includes as ActivityIncludes }
      : {}),
  }
}

function fromV2Post(post: Record<string, unknown>, includes?: ActivityIncludes): XPost | undefined {
  const id = typeof post.id === 'string' ? post.id : undefined
  const text = typeof post.text === 'string' ? post.text : undefined
  if (!id || !text) return undefined
  const authorId = typeof post.author_id === 'string' ? post.author_id : undefined
  const author = authorId ? includes?.users?.find((user) => user.id === authorId) : undefined
  return {
    id,
    text,
    ...(authorId ? { authorId } : {}),
    ...(author?.username ? { authorUsername: author.username } : {}),
    ...(typeof post.created_at === 'string' ? { createdAt: new Date(post.created_at).toISOString() } : {}),
    // The conversation is what tells a comment under one of the account's own
    // posts from a reply to the account out in somebody else's thread. Without
    // it the reply budget charged every reply to the latest own post.
    ...(typeof post.conversation_id === 'string' ? { conversationId: post.conversation_id } : {}),
    ...(typeof post.in_reply_to_user_id === 'string' ? { inReplyToUserId: post.in_reply_to_user_id } : {}),
  }
}

/**
 * Who did what to the account in a delivery, for the people record. Only
 * the X Activity API envelope carries enough to say: a post event names its
 * author, a follow its source, a like is credited to whoever `includes`
 * names that is not the account itself. The account's own doings are not
 * interactions.
 */
export function interactionsFromActivity(
  payload: Record<string, unknown>,
  botUserId: string,
): Array<{ userId: string; username?: string; kind: Interaction }> {
  const event = activityEvent(payload)
  if (!event) return []
  const users = event.includes?.users ?? []
  const named = (userId: string, kind: Interaction) => {
    if (!userId || userId === botUserId) return []
    const username = users.find((user) => user.id === userId)?.username
    return [{ userId, kind, ...(username ? { username } : {}) }]
  }
  const author = typeof event.payload.author_id === 'string' ? event.payload.author_id : ''
  switch (event.eventType) {
    case 'post.mention.create':
      return named(author, 'mention')
    case 'post.reply.create':
      return named(author, 'reply')
    case 'post.quote.create':
      return named(author, 'quote')
    case 'post.repost.create':
      return named(author, 'repost')
    case 'follow.follow': {
      const source = event.payload.source as { id?: string } | undefined
      return named(source?.id ?? '', 'follow')
    }
    case 'like.create': {
      const liker = users.find((user) => user.id && user.id !== botUserId)
      return named(liker?.id ?? '', 'like')
    }
    default:
      return []
  }
}

export function crcResponse(token: string, consumerSecret: string): string {
  const digest = createHmac('sha256', consumerSecret).update(token).digest('base64')
  return `sha256=${digest}`
}

export function isValidWebhookSignature(
  rawBody: Buffer,
  signature: string | undefined,
  consumerSecret: string,
): boolean {
  if (!signature) return false
  const expected = Buffer.from(crcResponse(rawBody.toString('utf8'), consumerSecret))
  const received = Buffer.from(signature)
  return expected.length === received.length && timingSafeEqual(expected, received)
}

export function activityEntries(payload: Record<string, unknown>): Array<{
  kind: string
  id: string
  payload: unknown
}> {
  const entries: Array<{ kind: string; id: string; payload: unknown }> = []
  const event = activityEvent(payload)
  if (event) {
    const id = typeof event.payload.id === 'string' ? event.payload.id : event.eventId ?? '0'
    return [{ kind: event.eventType, id, payload: event.payload }]
  }
  for (const [kind, value] of Object.entries(payload)) {
    if (!kind.endsWith('_events') || !Array.isArray(value)) continue
    for (const [index, event] of value.entries()) {
      const object = event && typeof event === 'object' ? (event as Record<string, unknown>) : {}
      const id = String(object.id ?? object.id_str ?? `${index}`)
      entries.push({ kind, id, payload: event })
    }
  }
  return entries
}

/**
 * Non-post activity only: follows, likes, reposts and the rest.
 *
 * Posts are deliberately excluded. They reach memory through the reply harness
 * instead, which stores them as a chat turn attributed to their author; writing
 * the raw delivery here as well would put the same conversation back into
 * memory as an unattributed blob under the account's own id.
 */
/** Likes, reposts, follows and deletions: counted, not remembered. */
export function isEngagement(kind: string): boolean {
  return /^(like|favorite|post\.repost|repost|retweet|follow|unfollow|block|mute|tweet_delete)[._]/.test(kind) ||
    /^(favorite|follow|block|mute|tweet_delete|retweet)_events$/.test(kind)
}

export async function captureWebhookActivity(
  memory: Memory,
  payload: Record<string, unknown>,
): Promise<void> {
  await Promise.all(
    activityEntries(payload)
      .filter((entry) => entry.kind !== 'tweet_create_events' && !POST_EVENTS.has(entry.kind))
      // Engagement -- likes, reposts, follows -- is counted in the people
      // store, which is what decides who gets answered. Stored here as raw
      // payloads it was two thirds of memory, and recall returned JSON about
      // who liked what instead of what anybody said.
      .filter((entry) => !isEngagement(entry.kind))
      .map((entry) => memory.captureActivity(entry.kind, entry.id, entry.payload)),
  )
}
