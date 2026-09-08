import { Client, OAuth1 } from '@xdevplatform/xdk'
import type {
  MentionWindow,
  XClient,
  XCreateOptions,
  XMedia,
  XPost,
  XSearchOptions,
  XTools,
} from '../types.js'

interface RawPost {
  id?: string
  text?: string
  authorId?: string
  conversationId?: string
  createdAt?: string
  inReplyToUserId?: string
}

interface RawUser {
  id?: string
  username?: string
}

function toPosts(
  data: RawPost[] | undefined,
  users: RawUser[] | undefined,
): XPost[] {
  const usernames = new Map(
    (users ?? []).flatMap((user) => (user.id && user.username ? [[user.id, user.username]] : [])),
  )
  return (data ?? []).flatMap((post) => {
    if (!post.id || !post.text) return []
    const username = post.authorId ? usernames.get(post.authorId) : undefined
    return [
      {
        id: post.id,
        text: post.text,
        ...(post.authorId ? { authorId: post.authorId } : {}),
        ...(username ? { authorUsername: username } : {}),
        ...(post.conversationId ? { conversationId: post.conversationId } : {}),
        ...(post.createdAt ? { createdAt: post.createdAt } : {}),
        ...(post.inReplyToUserId ? { inReplyToUserId: post.inReplyToUserId } : {}),
      },
    ]
  })
}

const POST_FIELDS = ['created_at', 'conversation_id', 'text'] as const

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
    return this.searchPosts(
      `(@${this.username} OR to:${this.username}) -from:${this.username} -is:retweet`,
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
      ...(options.sortOrder ? { sortOrder: options.sortOrder } : {}),
      postFields: [...POST_FIELDS],
      userFields: [...USER_FIELDS],
      expansions: ['author_id', 'in_reply_to_user_id'],
    })
    return toPosts(response.data, response.includes?.users)
  }

  async getPost(id: string): Promise<XPost | undefined> {
    const response = await this.readClient.posts.getById(id, {
      postFields: [...POST_FIELDS],
      userFields: [...USER_FIELDS],
      expansions: ['author_id', 'in_reply_to_user_id'],
    })
    const post = response.data
    return post ? toPosts([post], response.includes?.users)[0] : undefined
  }

  /** Replies/comments on a post, found through its conversation id. */
  async getConversation(conversationId: string, maxResults = 25): Promise<XPost[]> {
    return this.searchPosts(`conversation_id:${conversationId}`, { maxResults })
  }

  /** The account's own numeric id, resolved once and cached. */
  async userId(): Promise<string | undefined> {
    if (this.cachedUserId === undefined) {
      const response = await this.readClient.users.getByUsername(this.username)
      this.cachedUserId = response.data?.id ?? ''
    }
    return this.cachedUserId || undefined
  }

  private cachedUserId: string | undefined

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

  /**
   * Chunked media upload: INIT, one APPEND per chunk, FINALIZE. An animated GIF
   * is transcoded after FINALIZE, so the id is only attachable once processing
   * reports `succeeded`; attaching it earlier fails the post.
   */
  async uploadMedia(media: XMedia): Promise<{ mediaId: string }> {
    const initialized = await this.writeClient.media.initializeUpload({
      mediaType: media.mimeType,
      mediaCategory: media.mimeType === 'image/gif' ? 'tweet_gif' : 'tweet_video',
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
