import { createHmac, timingSafeEqual } from 'node:crypto'
import { isWorthAnswering, type Memory, type XPost } from '../types.js'

/**
 * Account Activity delivers v1.1-shaped tweet objects, not v2 ones: snake_case
 * `id_str` rather than `id`, the author nested under `user`, and the text
 * truncated into `extended_tweet.full_text` once it passes 140 characters.
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
  options: { botUserId?: string; botUsername: string },
): XPost[] {
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
    if (options.botUserId && post.authorId === options.botUserId) return []
    if (!isWorthAnswering(post, options)) return []
    return [post]
  })
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
export async function captureWebhookActivity(
  memory: Memory,
  payload: Record<string, unknown>,
): Promise<void> {
  await Promise.all(
    activityEntries(payload)
      .filter((entry) => entry.kind !== 'tweet_create_events')
      .map((entry) => memory.captureActivity(entry.kind, entry.id, entry.payload)),
  )
}
