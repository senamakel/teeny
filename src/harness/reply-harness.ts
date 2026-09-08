import {
  personOf,
  type LanguageModel,
  type Memory,
  type ReplyDraft,
  type XPost,
  type XTools,
} from '../types.js'

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
  ) {}

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
    const thread = await this.readThread(post)
    const conversation = thread
      .map(
        (entry) =>
          `${entry.authorUsername ? `@${entry.authorUsername}` : 'someone'}: ${entry.text}`,
      )
      .join('\n')

    // Two recalls: what is relevant to this post, and what the account itself
    // has been doing lately. The second is not a nicety. Asked "much higher for
    // tiny?", the account replied "I'm just a mascot who doesn't read charts"
    // -- hours after publishing a chart of its own token. It had no idea,
    // because nothing ever told it what it had posted.
    // Both recalls fail soft. A reply drafted without memory is a worse reply;
    // a reply that never goes out because the memory service returned a 401 is
    // a person on the timeline being ignored, and the mention does not come
    // back -- the queue marks it failed and nothing retries it.
    const [recalled, ownActivity] = await Promise.all([
      Promise.resolve(
        this.memory.recall(
          'What context, past interactions, product facts, and tone are relevant to replying to ' +
            `this X post? ${post.text}` +
            (conversation ? `\n\nThe conversation it sits in:\n${conversation}` : ''),
          person ? { person } : {},
        ),
      ).catch(() => ''),
      Promise.resolve(
        this.memory.recall(
          'What has this account itself posted recently? Include its daily chart posts, the ' +
            'market reads it published, and anything it has said publicly in the last few days.',
        ),
      ).catch(() => ''),
    ])

    const memory = ownActivity.trim()
      ? `${recalled}\n\nWHAT YOU YOURSELF HAVE POSTED RECENTLY:\n${ownActivity}`
      : recalled

    const draft = await this.model.reply({
      post,
      memory,
      ...(thread.length > 0 ? { thread } : {}),
    })

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

    const ordered = [...byId.values()].sort((a, b) => (BigInt(a.id) < BigInt(b.id) ? -1 : 1))
    if (ordered.length <= this.thread.maxPosts) return ordered

    // Over budget, the root and the most recent exchanges are what matter; the
    // middle of a long thread is the part a reply can most afford to miss.
    const [first, ...rest] = ordered
    return [...(first ? [first] : []), ...rest.slice(-(this.thread.maxPosts - 1))]
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
