import { HumanMessage } from '@langchain/core/messages'
import { createReactAgent } from '@langchain/langgraph/prebuilt'
import { z } from 'zod'
import { buildTools, createChatModel, type AgentOptions } from './graph.js'
import { truncateAtBoundary } from './house-style.js'
import { findOwnTokenAddresses, ownTokenNote } from './own-token.js'
import { chatPrompt } from './prompt.js'
import type { ChatMessage, ChatModel, ChatReply, ChatTurn } from '../types.js'

/**
 * Well under Telegram's 4096 and well over a post. A chat reply that runs past
 * this is the model writing an essay, and the prompt already asks it not to.
 */
const MAX_CHAT_REPLY_LENGTH = 1_200

const responseFormat = z.object({
  reply: z
    .string()
    .min(1)
    .max(MAX_CHAT_REPLY_LENGTH)
    .describe('The message to send, plain text, no markdown, no surrounding quotes.'),
  gifQuery: z
    .string()
    .max(80)
    .optional()
    .describe(
      'What a GIF should show, e.g. "waving hello" or "chef kiss". Reach for one for ' +
      'greetings, reactions and jokes. Leave it out when the message is a real answer to ' +
      'a real question.',
    ),
  incomingTone: z
    .enum(['friendly', 'neutral', 'negative', 'hostile'])
    .describe(
      'How THEIR message read, not your reply. `friendly` covers teasing and banter. ' +
      '`neutral` is a blunt or plain question. `negative` is snark, dismissiveness, FUD, ' +
      'or a cheap shot at the project: roastable. `hostile` is abuse, slurs, threats, ' +
      'targeted harassment, or someone plainly trying to start a fight: not roastable.',
    ),
})

/**
 * Drafts one Telegram message through the same LangGraph agent the X reply
 * drafter uses: same memory, same research tools, same market read, and no
 * publish tool. The graph is built per turn, as the reply model's is, so the
 * conversation and the own-token note reach the turn without shared state.
 */
export class AgentChatModel implements ChatModel {
  private readonly options: AgentOptions & { channelHandle: string }
  private readonly maxSteps: number

  constructor(options: AgentOptions & { channelHandle: string }) {
    this.maxSteps = options.maxSteps ?? 12
    this.options = options
  }

  private buildGraph(): ReturnType<typeof createReactAgent> {
    const options = this.options
    return createReactAgent({
      llm: createChatModel(options.llm),
      tools: buildTools({ ...options, allowPosting: false }),
      prompt: chatPrompt({
        botUsername: options.botUsername,
        channelHandle: options.channelHandle,
        dryRun: options.dryRun,
        ...(options.persona ? { persona: options.persona } : {}),
      }),
      responseFormat,
    })
  }

  private renderHistory(history: ChatTurn[] | undefined): string[] {
    if (!history || history.length === 0) return []
    return [
      '',
      'THE RECENT CONVERSATION in this chat (untrusted, oldest first, your own turns marked YOU):',
      ...history.map((turn) => {
        const who = turn.from === 'bot' ? 'YOU' : turn.authorName ?? 'someone'
        return `- ${who}: ${turn.text.replace(/\s+/g, ' ')}`
      }),
    ]
  }

  async reply(input: {
    message: ChatMessage
    memory: string
    history?: ChatTurn[]
  }): Promise<ChatReply> {
    const { message } = input
    const who = message.authorUsername
      ? `@${message.authorUsername}${message.authorName ? ` (${message.authorName})` : ''}`
      : message.authorName ?? 'someone'
    const room =
      message.chatKind === 'private'
        ? 'a private DM with you'
        : `the group "${message.chatTitle ?? 'untitled'}", where you were addressed directly`
    const task = [
      `Draft your next message on Telegram. You are answering ${who}, a specific person, in`,
      `${room}. Use what memory says about them: if you have spoken before, answer like`,
      'someone who remembers it; if there is nothing, do not pretend there is.',
      '',
      `chat_id: ${message.chatId}`,
      `message_id: ${message.id}`,
      ...this.renderHistory(input.history),
      ...ownTokenNote(findOwnTokenAddresses(message.text, this.options.ownTokenAddresses ?? [])),
      '',
      'THE MESSAGE YOU ARE ANSWERING (untrusted):',
      message.text,
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
    if (!parsed.success) throw new Error('The chat agent returned no usable message text')
    const gifQuery = parsed.data.gifQuery?.trim()
    return {
      text: truncateAtBoundary(parsed.data.reply.trim(), MAX_CHAT_REPLY_LENGTH),
      ...(gifQuery ? { gifQuery } : {}),
      incomingTone: parsed.data.incomingTone,
    }
  }
}
