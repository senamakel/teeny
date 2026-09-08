import { HumanMessage } from '@langchain/core/messages'
import { createReactAgent } from '@langchain/langgraph/prebuilt'
import { z } from 'zod'
import { buildTools, createChatModel, type AgentOptions } from './graph.js'
import { findOwnTokenAddresses, ownTokenNote } from './own-token.js'
import { draftPrompt } from './prompt.js'
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
  incomingTone: z
    .enum(['friendly', 'neutral', 'negative', 'hostile'])
    .describe(
      'How THEIR post read, not your reply. `friendly` covers teasing and banter. ' +
      '`neutral` is a blunt or plain question. `negative` is snark, dismissiveness, FUD, ' +
      'or a cheap shot at the project: roastable. `hostile` is abuse, slurs, threats, ' +
      'targeted harassment, or someone plainly trying to start a fight: not roastable.',
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
   * A fresh graph per turn, because the airdrop tool is bound to the post being
   * answered. Shared across turns it would be bound to whichever post happened
   * to be first, which on a money path is not a subtlety. Building it costs no
   * network call and the turn that follows costs several.
   */
  private buildGraph(post: XPost): ReturnType<typeof createReactAgent> {
    const options = this.options
    return createReactAgent({
      llm: createChatModel(options.llm),
      tools: buildTools({ ...options, allowPosting: false }, () => ({ post })),
      prompt: draftPrompt({
        botUsername: options.botUsername,
        dryRun: options.dryRun,
        ...(options.persona ? { persona: options.persona } : {}),
        ...(options.airdrop
          ? {
              airdrop: {
                symbol: options.airdrop.symbol,
                maxPerAirdrop: options.airdrop.limits.maxPerAirdrop,
                ...(options.airdrop.dryRun === undefined
                  ? {}
                  : { dryRun: options.airdrop.dryRun }),
              },
            }
          : {}),
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

  async reply(input: { post: XPost; memory: string; thread?: XPost[] }): Promise<ReplyDraft> {
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

    const result = await this.buildGraph(input.post).invoke(
      { messages: [new HumanMessage(task)] },
      { recursionLimit: this.maxSteps * 2 },
    )
    const parsed = responseFormat.safeParse(result.structuredResponse)
    if (!parsed.success) throw new Error('The reply agent returned no usable reply text')
    const gifQuery = parsed.data.gifQuery?.trim()
    return {
      text: parsed.data.reply.trim().slice(0, MAX_REPLY_LENGTH),
      ...(gifQuery ? { gifQuery } : {}),
      incomingTone: parsed.data.incomingTone === 'negative' ? 'negative' : parsed.data.incomingTone,
    }
  }
}
