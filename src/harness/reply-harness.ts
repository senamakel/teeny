import { personOf, type LanguageModel, type Memory, type ReplyDraft, type XPost } from '../types.js'

function isSour(tone: ReplyDraft['incomingTone']): boolean {
  return tone === 'negative' || tone === 'hostile'
}

export class ReplyHarness {
  constructor(
    private readonly memory: Memory,
    private readonly model: LanguageModel,
  ) {}

  async generate(post: XPost): Promise<ReplyDraft> {
    // Recall first, capture second. The other order puts the post being
    // answered into memory before the lookup, so the agent reads its own input
    // back as history and greets a stranger with "good to see you again".
    const person = personOf(post)
    const recalled = await this.memory.recall(
      `What context, past interactions, product facts, and tone are relevant to replying to this X post? ${post.text}`,
      person ? { person } : {},
    )
    const draft = await this.model.reply({ post, memory: recalled })

    // Sour and abusive posts are answered but not kept. Memory is what the
    // account carries into every future conversation, and an insult stored
    // today is context that colours a reply to someone else next week.
    if (!isSour(draft.incomingTone)) await this.memory.captureMention(post)
    return draft
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
