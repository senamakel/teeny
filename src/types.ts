export interface XPost {
  id: string
  text: string
  authorId?: string
  authorUsername?: string
  conversationId?: string
  createdAt?: string
  /** Absent when the post is not a reply. Present, it names the post's parent author. */
  inReplyToUserId?: string
}

/**
 * Whether a post is close enough to answer.
 *
 * One level means a direct mention, or a direct reply to something the account
 * itself posted. A reply to somebody else's reply is two levels down: the
 * account is a bystander in a conversation between other people, and joining it
 * is both noise and a way to get pulled into arguments that are not its own.
 */
export function isWithinOneLevel(post: XPost, botUserId?: string): boolean {
  if (!post.inReplyToUserId) return true
  // Unknown identity fails closed. This used to return true, which made every
  // reply in every thread "close enough" the moment the account's id could not
  // be resolved -- the widest possible answer to the narrowest question.
  if (!botUserId) return false
  return post.inReplyToUserId === botUserId
}

/**
 * Whether a human deliberately typed the account's handle, rather than X
 * carrying it along.
 *
 * Replying to somebody puts every earlier participant's handle at the front of
 * the text automatically, so a thread the account was tagged into once mentions
 * it in every subsequent reply, none of which are addressed to it. The leading
 * run of handles is that machinery; anything after it was typed on purpose.
 *
 *   "@someone @bot Rhyming is a bold choice"  -> inherited, not for the bot
 *   "@someone Oh hi fren @bot"                -> deliberate, answer it
 */
export function mentionsDeliberately(post: XPost, botUsername: string): boolean {
  const handle = botUsername.replace(/^@/, '').toLowerCase()
  // "RT @handle: ..." is a retweet, and the handle in it is the author being
  // quoted rather than anyone being addressed.
  if (/^RT @[A-Za-z0-9_]{1,15}:/.test(post.text)) return false
  const body = post.text.replace(/^(?:\s*@[A-Za-z0-9_]{1,15}\b)+/, '')
  return new RegExp(`@${handle}\\b`, 'i').test(body)
}

/**
 * The account answers a post when it is close by, or when someone deliberately
 * called it in from further away. Both cases are still a mention: X permits an
 * automated reply to a post that names the account, and nothing here replies to
 * a post that does not.
 */
export function isWorthAnswering(
  post: XPost,
  options: { botUserId?: string; botUsername: string },
): boolean {
  return (
    mentionsDeliberately(post, options.botUsername) ||
    isWithinOneLevel(post, options.botUserId)
  )
}

export interface BotState {
  sinceId?: string
  /**
   * When the mention search last ran, saved whether or not anything matched.
   * Without it a quiet period re-reads the whole 7-day window every cycle, and
   * X bills per post retrieved.
   */
  lastPolledAt?: string
  repliedPostIds: string[]
}

/**
 * X v2 rejects a search carrying both `since_id` and `start_time`, so a window
 * is one or the other, never both.
 */
export type MentionWindow = { maxResults?: number } & (
  | { sinceId: string; startTime?: never }
  | { startTime: string; sinceId?: never }
  | { sinceId?: never; startTime?: never }
)

export interface XClient {
  findMentions(window?: MentionWindow): Promise<XPost[]>
  reply(postId: string, text: string): Promise<{ id: string }>
}

/** The human on the other side of a conversation, as CortexDB knows them. */
export interface Person {
  /**
   * CortexDB reads the segment before the first colon as the actor type and
   * accepts only user, agent, service, or system, so the X id is namespaced
   * inside the identifier rather than in front of it. The numeric id is used
   * because a handle can change and an id cannot.
   */
  id: string
  username?: string
}

export function personOf(post: XPost): Person | undefined {
  if (!post.authorId) return undefined
  return {
    id: `user:x-${post.authorId}`,
    ...(post.authorUsername ? { username: post.authorUsername } : {}),
  }
}

export interface Memory {
  captureActivity(kind: string, id: string, payload: unknown, person?: Person): Promise<void>
  /** Stores a mention as a chat turn from the person who wrote it. */
  captureMention(post: XPost): Promise<void>
  recall(query: string, options?: { person?: Person }): Promise<string>
  captureReply(input: XPost, reply: string, postedId?: string): Promise<void>
}

/**
 * How the incoming post read, judged by the drafter that just answered it.
 *
 * `negative` and `hostile` are kept apart on purpose. Snark and FUD are worth
 * a roast; abuse is worth a shrug. Collapsing them means either roasting
 * someone who is actually abusive, which is a fight the account cannot win, or
 * politely absorbing snark, which is duller than the account should be.
 */
export type IncomingTone = 'friendly' | 'neutral' | 'negative' | 'hostile'

/** What the drafter produces for one mention: the text, and optionally a GIF to attach. */
export interface ReplyDraft {
  text: string
  gifQuery?: string
  /** Absent means unjudged, which is treated as safe to remember. */
  incomingTone?: IncomingTone
}

export interface LanguageModel {
  reply(input: { post: XPost; memory: string }): Promise<ReplyDraft>
}

export interface Gif {
  id: string
  title: string
  pageUrl: string
  /** Direct media URL, small enough for an X upload. */
  mediaUrl: string
  mimeType: 'image/gif' | 'video/mp4'
}

export interface GifSource {
  search(query: string, options?: { channelOnly?: boolean }): Promise<Gif[]>
  get(id: string): Promise<Gif | undefined>
  download(gif: Gif): Promise<XMedia>
}

export interface StateStore {
  load(): Promise<BotState>
  save(state: BotState): Promise<void>
}

export interface XSearchOptions {
  maxResults?: number
  sinceId?: string
  /** ISO 8601, within the last 7 days. Ignored by X when sinceId is also set. */
  startTime?: string
  sortOrder?: 'recency' | 'relevancy'
}

export interface XCreateOptions {
  replyToPostId?: string
  quotePostId?: string
  mediaIds?: string[]
}

export interface XMedia {
  data: Uint8Array
  mimeType: 'image/gif' | 'video/mp4'
}

/**
 * The wider slice of the X API the agent is allowed to reach for. `XClient`
 * stays the narrow surface the deterministic reply job depends on.
 */
export interface XTools {
  searchPosts(query: string, options?: XSearchOptions): Promise<XPost[]>
  getPost(id: string): Promise<XPost | undefined>
  getConversation(conversationId: string, maxResults?: number): Promise<XPost[]>
  createPost(text: string, options?: XCreateOptions): Promise<{ id: string }>
  uploadMedia(media: XMedia): Promise<{ mediaId: string }>
}

export interface AgentRunResult {
  output: string
  steps: Array<{ tool: string; input: unknown }>
}

export interface Agent {
  run(input: { task: string; threadId?: string }): Promise<AgentRunResult>
}
