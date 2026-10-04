import {
  personOf,
  type LanguageModel,
  type Memory,
  type ReplyArticle,
  type ReplyDraft,
  type XPost,
  type XTools,
} from '../types.js'
import type { ArticleStore } from '../article/store.js'
import type { ImageReader } from '../media/types.js'
import type { SafetyGate } from '../safety/gate.js'
import { TurnContext } from '../memory/context.js'

function isSour(tone: ReplyDraft['incomingTone']): boolean {
  return tone === 'negative' || tone === 'hostile'
}

export class ReplyHarness {
  constructor(
    private readonly memory: Memory,
    private readonly model: LanguageModel,
    /**
     * Reading the thread is deterministic rather than a tool the model may or
     * may not reach for.
     *
     * The prompt used to suggest calling x_get_comments "when the mention lacks
     * context", which leaves the judgement to the turn that has the least
     * context to make it with. It went wrong in public: somebody replied to one
     * of the account's own posts suggesting it buy more of its own token, and
     * the account -- seeing only that reply, with a contract address in it --
     * called it a shill. The root post was the whole story and nothing had
     * fetched it.
     *
     * Omit to disable. Every post retrieved is billed by X, and the floor on a
     * search is ten, so this is the most expensive thing a reply does.
     */
    private readonly thread?: { x: XTools; maxPosts: number },
    /**
     * Something that can look at the post's pictures. Omit and the drafter
     * answers the words alone, which is what it always did; a mention that
     * is only a screenshot then gets answered as if it were blank.
     */
    private readonly vision?: ImageReader,
    /**
     * Scrubs the thread. The post being answered has already passed the gate
     * by the time this runs; the thread around it has not, and a line in it
     * about something the account does not touch is a line the drafter does
     * not need to see. Lexicon only, so it costs nothing per line.
     */
    private readonly safety?: Pick<SafetyGate, 'screen'>,
    /**
     * Where the account's own articles are kept. A reply under the post that
     * carried an article is a reply to the article, and the post itself says
     * almost nothing about it -- a title and a card. With the store, the
     * whole article goes to the drafter. Omit and the drafter answers from
     * the card, as it did before there were articles.
     */
    private readonly articles?: Pick<ArticleStore, 'findByPostId'>,
    /**
     * What the turn starts with: exact history with the person, the account's
     * own recent posts from the ledger, and memory. Omit and memory alone is
     * asked both questions, which is how every turn worked before.
     */
    context?: TurnContext,
  ) {
    this.context = context ?? new TurnContext({ memory })
  }

  private readonly context: TurnContext

  async generate(post: XPost): Promise<ReplyDraft> {
    // Recall first, capture second. The other order puts the post being
    // answered into memory before the lookup, so the agent reads its own input
    // back as history and greets a stranger with "good to see you again".
    const person = personOf(post)

    // The thread is read before the recall, not after, so memory can be asked
    // about the conversation rather than about its last line. A reply deep in
    // a thread often says almost nothing alone -- "sure, go on" -- and a lookup
    // keyed on that finds nothing, where the same lookup keyed on the whole
    // exchange finds the person, the project, and what was promised earlier.
    const [thread, images, article] = await Promise.all([
      this.readThread(post),
      this.lookAt(post),
      this.articleUnder(post),
    ])
    const conversation = thread
      .map(
        (entry) =>
          `${entry.authorUsername ? `@${entry.authorUsername}` : 'someone'}: ${entry.text}`,
      )
      .join('\n')

    // What is relevant to this post, what was said to this person before, and
    // what the account itself has been doing lately. The last is not a nicety.
    // Asked "much higher for tiny?", the account replied "I'm just a mascot who
    // doesn't read charts" -- hours after publishing a chart of its own token.
    // It had no idea, because nothing ever told it what it had posted.
    // Every source fails soft, and says so. A reply drafted without memory is
    // a worse reply; a reply that never goes out because the memory service
    // returned a 401 is a person on the timeline being ignored, and the mention
    // does not come back -- the queue marks it failed and nothing retries it.
    const gathered = await this.context.gather({
      query:
        'What context, past interactions, product facts, and tone are relevant to replying to ' +
        `this X post? ${post.text}` +
        (article ? `\n\nIt is a reply under the account's own article "${article.title}": ${article.dek}` : '') +
        (conversation ? `\n\nThe conversation it sits in:\n${conversation}` : ''),
      ...(person ? { person } : {}),
      ...(post.authorId ? { authorId: post.authorId } : {}),
      postId: post.id,
    })
    const memory = gathered.text

    const drafted = await this.model.reply({
      post,
      memory,
      ...(thread.length > 0 ? { thread } : {}),
      ...(images.length > 0 ? { images } : {}),
      ...(article ? { article } : {}),
    })
    // What memory said goes on the draft for the journal: the page shows
    // "recalled" next to "thinking", and it should be what was actually
    // recalled, not the drafter's paraphrase of it.
    const draft: ReplyDraft = {
      ...drafted,
      recalled: gathered.recalled,
    }

    // Sour and abusive posts are answered but not kept. Memory is what the
    // account carries into every future conversation, and an insult stored
    // today is context that colours a reply to someone else next week.
    // Capture is best-effort for the same reason: the draft already exists, and
    // losing it to a storage error helps nobody. Wrapped in `Promise.resolve`
    // rather than chained directly, so a capture that returns nothing at all
    // cannot take the reply down on the way past.
    if (!isSour(draft.incomingTone)) {
      await Promise.resolve(this.memory.captureMention(post)).catch(() => undefined)
    }
    return draft
  }

  /**
   * The conversation this post sits in, oldest first.
   *
   * The root is fetched separately rather than relied on from the search: a
   * `conversation_id:` search only reaches back seven days, so a thread rooted
   * before that returns its replies and loses the post they are all about --
   * which is the one piece that cannot be inferred from the rest.
   */
  private async readThread(post: XPost): Promise<XPost[]> {
    if (!this.thread || this.thread.maxPosts <= 0) return []
    const conversationId = post.conversationId
    if (!conversationId || conversationId === post.id) return []

    const [root, replies] = await Promise.all([
      this.thread.x.getPost(conversationId).catch(() => undefined),
      this.thread.x.getConversation(conversationId, this.thread.maxPosts).catch(() => []),
    ])

    const byId = new Map<string, XPost>()
    for (const entry of [...(root ? [root] : []), ...replies]) byId.set(entry.id, entry)
    // The post being answered is already in the prompt in full; including it
    // here too just spends budget saying the same thing twice.
    byId.delete(post.id)

    const ordered = [...byId.values()]
      .filter((entry) => !this.safety || this.safety.screen(entry.text))
      .sort((a, b) => (BigInt(a.id) < BigInt(b.id) ? -1 : 1))
    if (ordered.length <= this.thread.maxPosts) return ordered

    // Over budget, the root and the most recent exchanges are what matter; the
    // middle of a long thread is the part a reply can most afford to miss.
    const [first, ...rest] = ordered
    return [...(first ? [first] : []), ...rest.slice(-(this.thread.maxPosts - 1))]
  }

  /**
   * The account's own article the post is replying under, if it is one.
   *
   * Looked up by the conversation root rather than by anything in the thread:
   * the post that carried the article is the root of every reply under it,
   * however deep, and the root id arrives with the mention, so this costs no
   * X call and does not depend on the thread being fetched. A store that
   * cannot be read is an article the drafter does not see, not a reply that
   * does not go out.
   */
  private async articleUnder(post: XPost): Promise<ReplyArticle | undefined> {
    if (!this.articles) return undefined
    const rootId = post.conversationId
    if (!rootId || rootId === post.id) return undefined
    const record = await Promise.resolve(this.articles.findByPostId(rootId)).catch(() => undefined)
    if (!record?.postId || !record.markdown.trim()) return undefined
    return {
      slug: record.slug,
      title: record.title,
      dek: record.dek,
      markdown: record.markdown,
      ...(record.publishedAt ? { publishedAt: record.publishedAt } : {}),
      postId: record.postId,
    }
  }

  /**
   * What is in the post's pictures, one line each. Every failure is a
   * picture the drafter does not get to see rather than a reply that does
   * not go out; a video contributes its preview frame, which is what X
   * exposes, and the description says so.
   */
  private async lookAt(post: XPost): Promise<string[]> {
    const reader = this.vision
    if (!reader || !post.media?.length) return []
    const looked = await Promise.all(
      post.media.slice(0, 4).map(async (item) => {
        const url = item.url ?? item.previewUrl
        if (!url) return undefined
        try {
          const description = await reader.describe({ url })
          return item.type === 'photo'
            ? description
            : `(a ${item.type === 'animated_gif' ? 'GIF' : 'video'}; this is its preview frame) ${description}`
        } catch {
          return undefined
        }
      }),
    )
    return looked.filter((entry): entry is string => Boolean(entry))
  }

  /** Skipped for the same reason the mention was: the exchange is not kept. */
  async remember(
    post: XPost,
    reply: string,
    postedId?: string,
    tone?: ReplyDraft['incomingTone'],
  ): Promise<void> {
    if (isSour(tone)) return
    await this.memory.captureReply(post, reply, postedId)
  }
}
