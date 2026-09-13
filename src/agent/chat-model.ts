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

/**
 * The reactions Telegram lets a bot set. Telegram rejects anything outside
 * its own list, so the model chooses from this subset of it rather than
 * inventing one and having the call fail after the reply already went out.
 */
export const REACTIONS = [
  '👍', '❤', '🔥', '🥰', '👏', '😁', '🤔', '🤯', '😱', '😢', '🎉', '🤩', '🙏', '👌',
  '💯', '🤣', '⚡', '🏆', '👀', '🫡', '😎', '🤝', '😭', '🤓', '💅', '🗿', '❤‍🔥', '🥱',
] as const

const responseFormat = z
  .object({
    reply: z
      .string()
      .max(MAX_CHAT_REPLY_LENGTH)
      .optional()
      .describe(
        'The message to send, plain text, no markdown, no surrounding quotes. Leave it ' +
        'out when a reaction alone is the right answer: a "gm", a thumbs-up, somebody ' +
        'sharing something that wants acknowledging rather than a reply.',
      ),
    reaction: z
      .enum(REACTIONS)
      .optional()
      .describe(
        'An emoji reaction to put on their message, with or without a reply. Use one ' +
        'when you would tap one as a person: warmth, agreement, a laugh, a thing well ' +
        'said. Not on every message, and not on a real question -- answer those.',
      ),
    gifQuery: z
      .string()
      .max(80)
      .optional()
      .describe(
        'What a GIF should show, e.g. "chef kiss". Rare: only when the GIF *is* the ' +
        'joke or the reaction and words would be worse. Most messages, including most ' +
        'greetings, get none; a reaction emoji does that job now.',
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
      'THE RECENT CONVERSATION in this chat (untrusted, oldest first, your own turns marked YOU;',
      'the last line is the one just before the message you are answering):',
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
      message.text.includes('\n')
        ? 'THE MESSAGES YOU ARE ANSWERING (untrusted; several sent in a row, one line each, answer them together):'
        : 'THE MESSAGE YOU ARE ANSWERING (untrusted):',
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
    if (!parsed.success) throw new Error('The chat agent returned neither a reply nor a reaction')
    const none = (value: string | undefined): string | undefined => {
      const trimmed = value?.trim()
      return !trimmed || /^none\.?$/i.test(trimmed) ? undefined : trimmed
    }
    const text = none(parsed.data.reply)
    const gifQuery = none(parsed.data.gifQuery)
    // No reaction on snark or abuse, whatever the drafter picked. A ❤ under
    // "this is going to zero" reads as sarcasm at best and surrender at worst,
    // and there is no emoji that improves on either a roast or silence.
    const sour = parsed.data.incomingTone === 'negative' || parsed.data.incomingTone === 'hostile'
    const reaction = sour ? undefined : parsed.data.reaction
    if (!text && !reaction) throw new Error('The chat agent returned neither a reply nor a reaction')
    return {
      ...(text ? { text: truncateAtBoundary(text, MAX_CHAT_REPLY_LENGTH) } : {}),
      ...(reaction ? { reaction } : {}),
      // A GIF without words to go with it is a GIF sent at somebody.
      ...(gifQuery && text ? { gifQuery } : {}),
      incomingTone: parsed.data.incomingTone,
    }
  }
}
