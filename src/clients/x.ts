import { fitImageForX } from '../media/fit.js'
import { Client, OAuth1 } from '@xdevplatform/xdk'
import type {
  MentionWindow,
  XClient,
  XCreateOptions,
  XMedia,
  XPost,
  XPostMetrics,
  XOwnPostMetrics,
  XSearchOptions,
  XTools,
  XTrend,
} from '../types.js'

interface RawPost {
  id?: string
  text?: string
  authorId?: string
  conversationId?: string
  createdAt?: string
  inReplyToUserId?: string
  attachments?: { mediaKeys?: string[] | null } | null
  publicMetrics?: {
    likeCount?: number
    replyCount?: number
    repostCount?: number
    quoteCount?: number
    bookmarkCount?: number
    impressionCount?: number
  } | null
}

interface RawUser {
  id?: string
  username?: string
}

interface RawMedia {
  mediaKey?: string
  type?: string
  url?: string
  previewImageUrl?: string
  altText?: string
}

function toPosts(
  data: RawPost[] | undefined,
  users: RawUser[] | undefined,
  media: RawMedia[] | undefined = undefined,
): XPost[] {
  const usernames = new Map(
    (users ?? []).flatMap((user) => (user.id && user.username ? [[user.id, user.username]] : [])),
  )
  const attachments = new Map(
    (media ?? []).flatMap((item) => (item.mediaKey ? [[item.mediaKey, item]] : [])),
  )
  return (data ?? []).flatMap((post) => {
    if (!post.id || !post.text) return []
    const username = post.authorId ? usernames.get(post.authorId) : undefined
    const attached = (post.attachments?.mediaKeys ?? []).flatMap((key) => {
      const item = attachments.get(key)
      if (!item?.type) return []
      return [
        {
          type: item.type,
          ...(item.url ? { url: item.url } : {}),
          ...(item.previewImageUrl ? { previewUrl: item.previewImageUrl } : {}),
          ...(item.altText ? { altText: item.altText } : {}),
        },
      ]
    })
    return [
      {
        id: post.id,
        text: post.text,
        ...(post.authorId ? { authorId: post.authorId } : {}),
        ...(username ? { authorUsername: username } : {}),
        ...(post.conversationId ? { conversationId: post.conversationId } : {}),
        ...(post.createdAt ? { createdAt: post.createdAt } : {}),
        ...(post.inReplyToUserId ? { inReplyToUserId: post.inReplyToUserId } : {}),
        ...(attached.length > 0 ? { media: attached } : {}),
        ...(post.publicMetrics ? { metrics: toMetrics(post.id, post.publicMetrics) } : {}),
      },
    ]
  })
}

function toMetrics(id: string, raw: NonNullable<RawPost['publicMetrics']>): XPostMetrics {
  return {
    id,
    likes: raw.likeCount ?? 0,
    replies: raw.replyCount ?? 0,
    reposts: raw.repostCount ?? 0,
    quotes: raw.quoteCount ?? 0,
    ...(raw.impressionCount !== undefined ? { impressions: raw.impressionCount } : {}),
    ...(raw.bookmarkCount !== undefined ? { bookmarks: raw.bookmarkCount } : {}),
  }
}

// The counters ride along on the same request: a field, not a post, so nothing extra is billed.
const POST_FIELDS = ['created_at', 'conversation_id', 'text', 'attachments', 'public_metrics'] as const
// Photos carry `url`; videos and GIFs only a preview frame. Both are enough to
// look at, and neither costs anything extra: media is an expansion, not a post.
const MEDIA_FIELDS = ['media_key', 'type', 'url', 'preview_image_url', 'alt_text'] as const

/** X accepts up to 5 MB per APPEND call; this stays well inside that. */
const UPLOAD_CHUNK_BYTES = 4 * 1024 * 1024
const UPLOAD_POLL_ATTEMPTS = 20
const USER_FIELDS = ['username'] as const

export class XApiClient implements XClient, XTools {
  private readonly readClient: Client
  private readonly writeClient: Client
  private readonly username: string

  constructor(options: {
    username: string
    apiKey: string
    apiSecret: string
    accessToken: string
    accessTokenSecret: string
    bearerToken?: string
  }) {
    this.username = options.username.replace(/^@/, '')
    const oauth1 = new OAuth1({
      apiKey: options.apiKey,
      apiSecret: options.apiSecret,
      callback: 'oob',
      accessToken: options.accessToken,
      accessTokenSecret: options.accessTokenSecret,
    })
    this.writeClient = new Client({ oauth1 })
    this.readClient = options.bearerToken
      ? new Client({ bearerToken: options.bearerToken })
      : this.writeClient
  }

  /**
   * Everything addressed to the account in the last 7 days: posts that mention
   * it, and replies to its own posts, which `to:` catches even when the client
   * did not put the handle in the text. Its own posts and reposts are excluded.
   */
  async findMentions(window: MentionWindow = {}): Promise<XPost[]> {
    // The live handle, not the configured one: `-from:` is what keeps the
    // account's own posts out of its own mention search, and a renamed account
    // excludes nothing (or, worse, excludes whoever took the old handle).
    const { username } = await this.identity()
    return this.searchPosts(
      `(@${username} OR to:${username}) -from:${username} -is:retweet`,
      {
        ...(window.maxResults ? { maxResults: window.maxResults } : {}),
        ...(window.sinceId ? { sinceId: window.sinceId } : {}),
        ...(window.startTime ? { startTime: window.startTime } : {}),
      },
    )
  }

  async searchPosts(query: string, options: XSearchOptions = {}): Promise<XPost[]> {
    const response = await this.readClient.posts.searchRecent(query, {
      // X's floor is 10, so asking for fewer still bills for 10.
      maxResults: Math.min(Math.max(options.maxResults ?? 10, 10), 100),
      ...(options.sinceId ? { sinceId: options.sinceId } : {}),
      ...(options.startTime ? { startTime: options.startTime } : {}),
      ...(options.endTime ? { endTime: options.endTime } : {}),
      ...(options.sortOrder ? { sortOrder: options.sortOrder } : {}),
      postFields: [...POST_FIELDS],
      userFields: [...USER_FIELDS],
      mediaFields: [...MEDIA_FIELDS],
      expansions: ['author_id', 'in_reply_to_user_id', 'attachments.media_keys'],
    })
    return toPosts(response.data, response.includes?.users, response.includes?.media)
  }

  async getPost(id: string): Promise<XPost | undefined> {
    const response = await this.readClient.posts.getById(id, {
      postFields: [...POST_FIELDS],
      userFields: [...USER_FIELDS],
      mediaFields: [...MEDIA_FIELDS],
      expansions: ['author_id', 'in_reply_to_user_id', 'attachments.media_keys'],
    })
    const post = response.data
    return post ? toPosts([post], response.includes?.users, response.includes?.media)[0] : undefined
  }

  /**
   * What X says is trending, worldwide (WOEID 1) unless told a place. Not
   * every access tier has the endpoint; a refusal is an empty list, and the
   * caller reads the timeline without it.
   */
  async trends(options: { woeid?: number; max?: number } = {}): Promise<XTrend[]> {
    const response = await this.readClient.trends.getByWoeid(options.woeid ?? 1, {
      maxTrends: Math.min(Math.max(options.max ?? 20, 1), 50),
      trendFields: ['trend_name', 'tweet_count'],
    })
    return (response.data ?? []).flatMap((trend) => {
      const name = trend.trendName?.trim()
      if (!name) return []
      return [{ name, ...(trend.tweetCount !== undefined ? { posts: trend.tweetCount } : {}) }]
    })
  }

  /** Replies/comments on a post, found through its conversation id. */
  async getConversation(conversationId: string, maxResults = 25): Promise<XPost[]> {
    return this.searchPosts(`conversation_id:${conversationId}`, { maxResults })
  }

  /** The account's own numeric id, resolved once and cached. */
  /**
   * The account these credentials act as, read from the token rather than
   * looked up by handle.
   *
   * A handle is a mutable label: the account renamed itself from @TinyMascot to
   * @teenydroid, the configured name stopped resolving, and every self-check
   * keyed on the resulting id went quiet at once — the account answered its own
   * post, then answered that answer. The token cannot go stale that way.
   *
   * This goes through the OAuth 1.0a client on purpose. `/2/users/me` is asking
   * "who am I", which the app-only bearer cannot answer: it identifies the app,
   * not the account.
   */
  async identity(): Promise<{ id: string; username: string }> {
    if (!this.cachedIdentity) {
      const response = await this.writeClient.users.getMe({ userFields: ['username'] })
      const id = response.data?.id
      if (!id) {
        throw new Error(
          `X did not identify the authenticated account: ${JSON.stringify(response.errors ?? response)}`,
        )
      }
      this.cachedIdentity = { id, username: response.data?.username ?? this.username }
    }
    return this.cachedIdentity
  }

  /**
   * Throws rather than answering "unknown". Every caller uses this to tell the
   * account's own posts from everybody else's, and an undefined answer used to
   * disable that check silently instead of stopping the run.
   */
  async userId(): Promise<string | undefined> {
    return (await this.identity()).id
  }

  /**
   * How the account and this person are connected, as X sees it.
   *
   * `followed_by` is them following the account; `following` is the account
   * following them. They are not the same question and only the first one is
   * evidence of anything, since anyone can be followed by a bot farm.
   *
   * This needs the OAuth 1.0a client: `connection_status` is a relationship
   * between two accounts, so an app-only token has nobody to be the other half
   * of it and X simply omits the field rather than failing.
   *
   * Returns undefined when the answer cannot be established -- a deleted user,
   * a rate limit, an outage -- which callers must not read as "no".
   */
  async followsAccount(userId: string): Promise<boolean | undefined> {
    try {
      const response = await this.writeClient.users.getById(userId, {
        userFields: ['connection_status'],
      })
      const status = response.data?.connectionStatus
      if (!status) return undefined
      return status.includes('followed_by')
    } catch {
      return undefined
    }
  }

  private cachedIdentity: { id: string; username: string } | undefined

  async createPost(text: string, options: XCreateOptions = {}): Promise<{ id: string }> {
    const response = await this.writeClient.posts.create({
      text,
      ...(options.replyToPostId
        ? { reply: { inReplyToTweetId: options.replyToPostId } }
        : {}),
      ...(options.quotePostId ? { quoteTweetId: options.quotePostId } : {}),
      ...(options.mediaIds?.length ? { media: { mediaIds: options.mediaIds } } : {}),
    })
    if (!response.data?.id) {
      throw new Error(`X did not return a post id: ${JSON.stringify(response.errors)}`)
    }
    return { id: response.data.id }
  }

  async reply(postId: string, text: string): Promise<{ id: string }> {
    return this.createPost(text, { replyToPostId: postId })
  }

  async likePost(postId: string): Promise<void> {
    const { id } = await this.identity()
    const response = await this.writeClient.users.likePost(id, { tweetId: postId })
    if (response.errors?.length) throw new Error(`X refused the like: ${JSON.stringify(response.errors)}`)
  }

  /**
   * Counters for up to a hundred posts at a time. Through the account's own
   * client rather than the app's: impression counts are the account's to
   * see on its own posts, and the bearer is not the account.
   */
  async postMetrics(ids: string[]): Promise<XOwnPostMetrics[]> {
    const metrics: XOwnPostMetrics[] = []
    for (let offset = 0; offset < ids.length; offset += 100) {
      const response = await this.writeClient.posts.getByIds(ids.slice(offset, offset + 100), {
        postFields: ['public_metrics', 'created_at'],
      })
      for (const post of response.data ?? []) {
        const counts = post.publicMetrics
        if (!post.id || !counts) continue
        metrics.push({
          id: post.id,
          impressions: counts.impressionCount ?? 0,
          likes: counts.likeCount ?? 0,
          replies: counts.replyCount ?? 0,
          reposts: counts.repostCount ?? 0,
          quotes: counts.quoteCount ?? 0,
        })
      }
    }
    return metrics
  }

  /**
   * Chunked media upload: INIT, one APPEND per chunk, FINALIZE. An animated GIF
   * is transcoded after FINALIZE, so the id is only attachable once processing
   * reports `succeeded`; attaching it earlier fails the post.
   */
  async uploadMedia(uploaded: XMedia): Promise<{ mediaId: string }> {
    // A generated still can be far over the 5 MB image limit; see fit.ts.
    const media = await fitImageForX(uploaded)
    const initialized = await this.writeClient.media.initializeUpload({
      mediaType: media.mimeType,
      // By category, not by MIME type: X puts a still image, an animation and
      // a video through three different pipelines and rejects the post, not
      // the upload, when one lands in the wrong one.
      mediaCategory:
        media.mimeType === 'image/gif'
          ? 'tweet_gif'
          : media.mimeType.startsWith('image/')
            ? 'tweet_image'
            : 'tweet_video',
      totalBytes: media.data.byteLength,
    })
    const mediaId = initialized.data?.id
    if (!mediaId) {
      throw new Error(`X did not return a media id: ${JSON.stringify(initialized.errors)}`)
    }

    for (
      let offset = 0, segment = 0;
      offset < media.data.byteLength;
      offset += UPLOAD_CHUNK_BYTES, segment += 1
    ) {
      const chunk = media.data.subarray(offset, offset + UPLOAD_CHUNK_BYTES)
      await this.writeClient.media.appendUpload(mediaId, {
        media: Buffer.from(chunk).toString('base64'),
        segmentIndex: segment,
      })
    }

    const finalized = await this.writeClient.media.finalizeUpload(mediaId)
    await this.awaitProcessing(mediaId, finalized.data?.processingInfo)
    return { mediaId }
  }

  private async awaitProcessing(
    mediaId: string,
    info: { state?: string; checkAfterSecs?: number } | undefined,
  ): Promise<void> {
    let state = info?.state
    let wait = info?.checkAfterSecs ?? 1

    for (let attempt = 0; attempt < UPLOAD_POLL_ATTEMPTS; attempt += 1) {
      if (!state || state === 'succeeded') return
      if (state === 'failed') throw new Error(`X failed to process media ${mediaId}`)

      await new Promise((resolve) => setTimeout(resolve, Math.min(wait, 10) * 1_000))
      const status = await this.writeClient.media.getUploadStatus(mediaId)
      const next = status.data?.processingInfo
      state = next?.state
      wait = next?.checkAfterSecs ?? wait
    }
    throw new Error(`X is still processing media ${mediaId} after ${UPLOAD_POLL_ATTEMPTS} checks`)
  }
}
