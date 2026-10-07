import {
  cut,
  recordQuietly,
  sha256,
  tokensFromUsage,
  type InferenceLedger,
  type LedgerMessage,
} from './ledger.js'

/** A chat completion as the wire returns it, with the parts the ledger reads. */
export interface CompletionResponse {
  id?: string
  model?: string
  choices?: Array<{
    message?: {
      content?: string | null | Array<{ type?: string; text?: string }>
      reasoning_content?: string | null
    }
  }>
  usage?: unknown
}

export interface CompletionMessage {
  role: string
  /** A string, or content parts; an image part is hashed as the URL it carries. */
  content: string | Array<Record<string, unknown>>
}

function textOf(content: CompletionMessage['content']): string {
  if (typeof content === 'string') return content
  return content
    .map((part) => {
      if (typeof part.text === 'string') return part.text
      if (part.type === 'image_url') return '[image]'
      return ''
    })
    .join('')
}

/**
 * Writes a ledger row for a chat completion made with plain `fetch`, the way
 * the vision reader, the morning text model and the simple reply model make
 * theirs. Same shape as a graph step's row, so the page tells them apart by
 * source alone.
 */
export async function recordCompletion(
  ledger: InferenceLedger | undefined,
  call: {
    source: string
    model: string
    messages: CompletionMessage[]
    response: CompletionResponse
    startedAt: number
  },
): Promise<void> {
  if (!ledger) return
  const { messages, response } = call
  const choice = response.choices?.[0]?.message
  const output =
    typeof choice?.content === 'string'
      ? choice.content
      : (choice?.content ?? []).map((part) => part.text ?? '').join('')
  const reasoning = choice?.reasoning_content ?? undefined
  const input: LedgerMessage[] = messages
    .filter((message) => message.role !== 'system')
    .map((message) => ({ role: message.role === 'assistant' ? 'assistant' : 'user', text: cut(textOf(message.content)) }))
  const system = messages.filter((message) => message.role === 'system').map((m) => textOf(m.content))
  await recordQuietly(ledger, {
    lane: 'mascot',
    source: call.source,
    model: response.model ?? call.model,
    input,
    ...(reasoning ? { reasoning: cut(reasoning, 4_000) } : {}),
    output: cut(output, 4_000),
    tokens: tokensFromUsage(response.usage),
    ...(response.id ? { responseId: response.id } : {}),
    digest: sha256(JSON.stringify(messages), JSON.stringify({ content: output, reasoning: reasoning ?? null })),
    ...(system.length ? { promptDigest: sha256(...system) } : {}),
    durationMs: Date.now() - call.startedAt,
  })
}
