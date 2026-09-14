import { HumanMessage, type BaseMessage } from '@langchain/core/messages'
import { createReactAgent } from '@langchain/langgraph/prebuilt'
import { z } from 'zod'
import { buildTools, createChatModel, producedMediaId, type AgentOptions } from './graph.js'
import { truncateAtBoundary } from './house-style.js'
import { findOwnTokenAddresses, ownTokenNote } from './own-token.js'
import { chatPrompt } from './prompt.js'
import { MEDIA_ID_PATTERN } from './tools/media.js'
import { gateMedia } from '../media/wiring.js'
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
    // Lenient here, checked in code: a model that writes "none" or "" for an
    // empty slot must not fail the whole structured answer over it.
    mediaId: z
      .string()
      .optional()
      .describe(
        'The mediaId returned by generate_image or generate_video THIS turn, to send it ' +
        'with the reply. Only an id a tool actually returned; leave it out otherwise.',
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
    // Eyes, no sketchbook. On Telegram pictures arrive on the calendar's
    // schedule, never on request: a group can ask a bot to draw all day, and
    // every one of those is a bill and a post the operator did not plan.
    const media = gateMedia(options.media, {})
    return createReactAgent({
      llm: createChatModel({ source: 'chat', ...options.llm }),
      tools: buildTools({ ...options, allowPosting: false, ...(media ? { media } : {}) }),
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
    const mediaId = none(parsed.data.mediaId)
    const generated =
      mediaId !== undefined &&
      MEDIA_ID_PATTERN.test(mediaId) &&
      producedMediaId(result.messages as BaseMessage[], mediaId)
    // A GIF next to a picture it just drew is two attachments for one line.
    const gifQuery = generated ? undefined : none(parsed.data.gifQuery)
    // No reaction on snark or abuse, whatever the drafter picked. A ❤ under
    // "this is going to zero" reads as sarcasm at best and surrender at worst,
    // and there is no emoji that improves on either a roast or silence.
    const sour = parsed.data.incomingTone === 'negative' || parsed.data.incomingTone === 'hostile'
    const reaction = sour ? undefined : parsed.data.reaction
    if (!text && !reaction && !generated) {
      throw new Error('The chat agent returned neither a reply nor a reaction')
    }
    return {
      ...(text ? { text: truncateAtBoundary(text, MAX_CHAT_REPLY_LENGTH) } : {}),
      ...(reaction ? { reaction } : {}),
      // A GIF without words to go with it is a GIF sent at somebody.
      ...(gifQuery && text ? { gifQuery } : {}),
      // A picture somebody asked for can stand alone; it is the answer.
      ...(generated && mediaId ? { mediaId } : {}),
      incomingTone: parsed.data.incomingTone,
    }
  }
}
