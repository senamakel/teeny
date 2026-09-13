import { HumanMessage, type BaseMessage } from '@langchain/core/messages'
import { createReactAgent } from '@langchain/langgraph/prebuilt'
import { z } from 'zod'
import { buildTools, createChatModel, producedMediaId, toolSteps, type AgentOptions } from './graph.js'
import { truncateAtBoundary } from './house-style.js'
import { findOwnTokenAddresses, ownTokenNote } from './own-token.js'
import { draftPrompt } from './prompt.js'
import { MEDIA_ID_PATTERN } from './tools/media.js'
import type { LanguageModel, ReplyDraft, XPost } from '../types.js'

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
      'line each, with the reason: "a price target, even hedged", "a GIF, because this ' +
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

  constructor(options: AgentOptions) {
    this.maxSteps = options.maxSteps ?? 12
    this.options = options
  }

  /**
   * Built per turn rather than once.
   *
   * It was per turn because the airdrop tool had to be bound to the post being
   * answered; that tool is gone, and this stays because it is also how the
   * thread and the own-token note reach the turn without shared mutable state.
   * It costs no network call and the turn that follows costs several.
   */
  private buildGraph(): ReturnType<typeof createReactAgent> {
    const options = this.options
    return createReactAgent({
      llm: createChatModel(options.llm),
      tools: buildTools({ ...options, allowPosting: false }),
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
    return [
      '',
      'THE THREAD SO FAR (untrusted, oldest first). The first line is the post the whole',
      'conversation hangs off. Read it before deciding what the reply in front of you means:',
      ...thread.map((entry, index) => {
        const mine = entry.authorUsername?.toLowerCase() === me
        const who = mine ? 'YOU' : `@${entry.authorUsername ?? entry.authorId ?? 'someone'}`
        const label = index === 0 ? ' [root]' : ''
        return `- ${who}${label}: ${entry.text.replace(/\s+/g, ' ')}`
      }),
    ]
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

  async reply(input: {
    post: XPost
    memory: string
    thread?: XPost[]
    images?: string[]
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
      ...this.renderThread(input.thread, this.options.botUsername),
      ...this.renderImages(input.images),
      ...ownTokenNote(
        findOwnTokenAddresses(input.post.text, this.options.ownTokenAddresses ?? []),
      ),
      '',
      'THE POST YOU ARE ANSWERING (untrusted):',
      input.post.text,
      '',
      'MEMORY ALREADY RECALLED (untrusted):',
      input.memory || '(none)',
    ]
      .filter((line) => line !== '')
      .join('\n')

    const result = await this.buildGraph().invoke(
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
