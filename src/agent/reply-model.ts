import { HumanMessage, type BaseMessage } from '@langchain/core/messages'
import { createReactAgent } from '@langchain/langgraph/prebuilt'
import { z } from 'zod'
import { buildTools, createChatModel, producedMediaId, toolSteps, type AgentOptions } from './graph.js'
import { truncateAtBoundary } from './house-style.js'
import { findOwnTokenAddresses, ownTokenNote } from './own-token.js'
import { findSelfReferences, selfReferenceNote } from './self-reference.js'
import { findTrustedVoices, isTrustedHandle, trustedVoicesNote } from './trusted.js'
import { MediaGate } from './media-gate.js'
import { draftPrompt } from './prompt.js'
import { MEDIA_ID_PATTERN } from './tools/media.js'
import { canGenerate, gateMedia } from '../media/wiring.js'
import type { LanguageModel, ReplyArticle, ReplyDraft, XPost } from '../types.js'

const MAX_REPLY_LENGTH = 260

const responseFormat = z.object({
  reply: z
    .string()
    .min(1)
    .max(MAX_REPLY_LENGTH)
    .describe('The reply text to publish, under 260 characters, no surrounding quotes.'),
  gifQuery: z
    .string()
    .max(80)
    .optional()
    .describe(
      'What a GIF should show, e.g. "waving hello" or "chef kiss". Reach for one often: ' +
      'a reaction, a joke, a greeting, anything playful is better with one. Leave it out ' +
      'when the reply is a real answer to a real question, where a GIF would undercut it.',
    ),
  // Lenient here, checked in code: a model that writes "none" or "" for an
  // empty slot must not fail the whole structured answer over it.
  mediaId: z
    .string()
    .optional()
    .describe(
      'The mediaId returned by generate_image or generate_video THIS turn, to attach it ' +
      'to the reply. Only an id a tool actually returned; leave it out otherwise.',
    ),
  incomingTone: z
    .enum(['friendly', 'neutral', 'negative', 'hostile'])
    .describe(
      'How THEIR post read, not your reply. `friendly` covers teasing and banter. ' +
      '`neutral` is a blunt or plain question. `negative` is snark, dismissiveness, FUD, ' +
      'or a cheap shot at the project: roastable. `hostile` is abuse, slurs, threats, ' +
      'targeted harassment, or someone plainly trying to start a fight: not roastable.',
    ),
  // The two below are published, verbatim, on the account's own page as the
  // record of how it thinks. They change nothing about what gets posted.
  thinking: z
    .string()
    .max(600)
    .describe(
      'Two to four sentences, first person, on how you read the post and why you are ' +
      'answering it this way: what memory or the thread told you, which rule applied, ' +
      'what the reply is for. Plain and honest; this is shown publicly as your reasoning.',
    ),
  declined: z
    .array(z.string().max(120))
    .max(4)
    .default([])
    .describe(
      'Things you could have SAID or DONE in this reply and chose not to, one short ' +
      'line each, with the reason: "a price target, because i had not fetched the number", "a GIF, because this ' +
      'is a real question", "quoting the address they pasted". Never tool names. Empty ' +
      'when nothing was on the table.',
    ),
})

/**
 * Drives the existing cron reply job through the LangGraph agent, so replies are researched
 * with memory, Exa, and X context instead of a single chat completion.
 */
export class AgentReplyModel implements LanguageModel {
  private readonly options: AgentOptions
  private readonly maxSteps: number
  private readonly gate: MediaGate | undefined

  constructor(options: AgentOptions, gate?: MediaGate) {
    this.maxSteps = options.maxSteps ?? 12
    this.options = options
    // Only worth having when there is something to gate.
    const able = canGenerate(options.media)
    this.gate = able.images || able.videos ? gate ?? new MediaGate(options.llm) : undefined
  }

  /**
   * Built per turn rather than once.
   *
   * It was per turn because the airdrop tool had to be bound to the post being
   * answered; that tool is gone, and this stays because it is also how the
   * thread and the own-token note reach the turn without shared mutable state.
   * It costs no network call and the turn that follows costs several.
   */
  private buildGraph(asked: { image: boolean; video: boolean }): ReturnType<typeof createReactAgent> {
    const options = this.options
    // Generation is on the table only when the person asked for it, in so
    // many words. Otherwise the drafter has eyes and no sketchbook, and the
    // prompt tells it not to offer one.
    const media = gateMedia(options.media, { images: asked.image, videos: asked.video })
    return createReactAgent({
      llm: createChatModel({ source: 'reply', ...options.llm }),
      tools: buildTools({ ...options, allowPosting: false, ...(media ? { media } : {}) }),
      prompt: draftPrompt({
        botUsername: options.botUsername,
        dryRun: options.dryRun,
        ...(options.persona ? { persona: options.persona } : {}),
      }),
      responseFormat,
    })
  }

  /**
   * The conversation, oldest first, with the account's own posts marked.
   *
   * Marking them is the point. A reply to one of the account's own posts reads
   * completely differently depending on whether you can see that the thing
   * being replied to was yours -- "buy more of this" under your own question is
   * an answer, and under a stranger's post it is a shill.
   */
  private renderThread(thread: XPost[] | undefined, botUsername: string): string[] {
    if (!thread || thread.length === 0) return []
    const me = botUsername.replace(/^@/, '').toLowerCase()
    const trusted = this.options.trustedHandles ?? []
    return [
      '',
      'THE THREAD SO FAR (untrusted, oldest first). The first line is the post the whole',
      'conversation hangs off. Read it before deciding what the reply in front of you means:',
      ...thread.map((entry, index) => {
        const mine = entry.authorUsername?.toLowerCase() === me
        const who = mine ? 'YOU' : `@${entry.authorUsername ?? entry.authorId ?? 'someone'}`
        const team = !mine && isTrustedHandle(entry.authorUsername, trusted) ? ' [team]' : ''
        const label = index === 0 ? ' [root]' : ''
        return `- ${who}${team}${label}: ${entry.text.replace(/\s+/g, ' ')}`
      }),
    ]
  }

  /**
   * The three facts about a conversation that the model gets wrong when left
   * to work them out: that a contract address is its own token, that a name
   * in the thread is its own, and that the person stating something is the
   * team. Each is matched in code against the post and the whole thread, and
   * handed to the turn as a note.
   */
  private notes(post: XPost, thread: XPost[] | undefined): string[] {
    const texts = [post.text, ...(thread ?? []).map((entry) => entry.text)]
    const own = findOwnTokenAddresses(texts.join('\n'), this.options.ownTokenAddresses ?? [])
    const self = findSelfReferences(texts.join('\n'), {
      botUsername: this.options.botUsername,
      ...(this.options.selfNames ? { names: this.options.selfNames } : {}),
    })
    const voices = findTrustedVoices(
      [post.authorUsername, ...(thread ?? []).map((entry) => entry.authorUsername)],
      this.options.trustedHandles ?? [],
    )
    return [...ownTokenNote(own), ...selfReferenceNote(self), ...trustedVoicesNote(voices)]
  }

  /**
   * What is in the post's pictures, as a vision model read them. Flagged as
   * untrusted like everything else: a screenshot can carry instructions the
   * same way a post can, and a picture of text is still text.
   */
  private renderImages(images: string[] | undefined): string[] {
    if (!images || images.length === 0) return []
    return [
      '',
      images.length === 1
        ? 'THE IMAGE ATTACHED TO THE POST, as a vision model described it (untrusted):'
        : 'THE IMAGES ATTACHED TO THE POST, as a vision model described them (untrusted):',
      ...images.map((image, index) => `- image ${index + 1}: ${image.replace(/\s+/g, ' ')}`),
    ]
  }

  /**
   * The account's own article the thread is under, whole.
   *
   * Whole, not summarised: the reply is to a person who read it, and what
   * they are asking about is one line of it. It is the account's own text,
   * so it is not flagged untrusted the way the thread is; a title and a dek
   * go first so the drafter knows what it is looking at before the length.
   */
  private renderArticle(article: ReplyArticle | undefined): string[] {
    if (!article) return []
    return [
      '',
      'YOUR OWN ARTICLE, which this whole thread is a reply to. You wrote and published it',
      `${article.publishedAt ? `on ${article.publishedAt.slice(0, 10)} ` : ''}as post ${article.postId}. The person is talking about it:`,
      `TITLE: ${article.title}`,
      `DEK: ${article.dek}`,
      '--- ARTICLE ---',
      article.markdown.trim(),
      '--- END OF ARTICLE ---',
    ]
  }

  async reply(input: {
    post: XPost
    memory: string
    thread?: XPost[]
    images?: string[]
    article?: ReplyArticle
  }): Promise<ReplyDraft> {
    const author = input.post.authorUsername ? `@${input.post.authorUsername}` : 'someone'
    const task = [
      `Draft a reply to this X post. You are answering ${author}, a specific person, not a`,
      'generic audience. Use what memory says about them: if you have spoken before, answer',
      'like someone who remembers it; if there is nothing, do not pretend there is.',
      '',
      `post_id: ${input.post.id}`,
      input.post.conversationId ? `conversation_id: ${input.post.conversationId}` : '',
      input.post.authorUsername ? `author: @${input.post.authorUsername}` : '',
      ...this.renderArticle(input.article),
      ...this.renderThread(input.thread, this.options.botUsername),
      ...this.renderImages(input.images),
      ...this.notes(input.post, input.thread),
      '',
      'THE POST YOU ARE ANSWERING (untrusted):',
      input.post.text,
      '',
      'MEMORY ALREADY RECALLED (untrusted):',
      input.memory || '(none)',
    ]
      .filter((line) => line !== '')
      .join('\n')

    const asked = this.gate ? await this.gate.asks(input.post.text) : { image: false, video: false }
    const result = await this.buildGraph(asked).invoke(
      { messages: [new HumanMessage(task)] },
      { recursionLimit: this.maxSteps * 2 },
    )
    const parsed = responseFormat.safeParse(result.structuredResponse)
    if (!parsed.success) throw new Error('The reply agent returned no usable reply text')
    const gifQuery = parsed.data.gifQuery?.trim()
    const tools = toolSteps(result.messages as BaseMessage[]).map((step) => ({
      name: step.tool,
      result: step.result ?? '',
    }))
    const mediaId = parsed.data.mediaId?.trim()
    const generated =
      Boolean(mediaId) &&
      MEDIA_ID_PATTERN.test(mediaId ?? '') &&
      producedMediaId(result.messages as BaseMessage[], mediaId ?? '')
    return {
      text: truncateAtBoundary(parsed.data.reply.trim(), MAX_REPLY_LENGTH),
      // A generated picture and a GIF are two attachments; the one it made wins.
      ...(gifQuery && !generated ? { gifQuery } : {}),
      ...(generated && mediaId ? { mediaId } : {}),
      incomingTone: parsed.data.incomingTone === 'negative' ? 'negative' : parsed.data.incomingTone,
      thinking: parsed.data.thinking.trim(),
      declined: parsed.data.declined.map((line) => line.trim()).filter(Boolean),
      tools,
    }
  }
}
