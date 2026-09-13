import {
  chatPersonOf,
  type ChatMemory,
  type ChatMessage,
  type ChatModel,
  type ChatReply,
  type ChatTurn,
} from '../types.js'

function isSour(tone: ChatReply['incomingTone']): boolean {
  return tone === 'negative' || tone === 'hostile'
}

/**
 * One Telegram turn: recall, draft, capture. The chat-shaped twin of
 * `ReplyHarness`, with the same order and the same reasons for it.
 */
export class ChatHarness {
  constructor(
    private readonly memory: ChatMemory,
    private readonly model: ChatModel,
  ) {}

  async generate(message: ChatMessage, history: ChatTurn[] = []): Promise<ChatReply> {
    // Recall first, capture second, or the turn reads its own input back as
    // history and greets a stranger like an old friend.
    const person = chatPersonOf(message)
    const conversation = history
      .map((turn) => `${turn.from === 'bot' ? 'you' : turn.authorName ?? 'someone'}: ${turn.text}`)
      .join('\n')

    // Both recalls fail soft: a message drafted without memory is a worse
    // message, and one that never goes out because the memory service is down
    // is a person in a DM being ignored.
    const [recalled, ownActivity] = await Promise.all([
      Promise.resolve(
        this.memory.recall(
          'What context, past interactions, product facts, and tone are relevant to answering ' +
            `this Telegram message? ${message.text}` +
            (conversation ? `\n\nThe conversation it sits in:\n${conversation}` : ''),
          { person },
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
      message,
      memory,
      ...(history.length > 0 ? { history } : {}),
    })

    // Sour and abusive messages are answered but not kept, for the same reason
    // as on X: memory is carried into every future conversation.
    if (!isSour(draft.incomingTone)) {
      await Promise.resolve(this.memory.captureChatMessage(message)).catch(() => undefined)
    }
    return draft
  }

  async remember(
    message: ChatMessage,
    reply: string,
    sentId?: string,
    tone?: ChatReply['incomingTone'],
  ): Promise<void> {
    if (isSour(tone)) return
    await this.memory.captureChatReply(message, reply, sentId)
  }
}
