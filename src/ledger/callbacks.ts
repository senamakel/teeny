import { BaseCallbackHandler } from '@langchain/core/callbacks/base'
import type { BaseMessage } from '@langchain/core/messages'
import type { LLMResult } from '@langchain/core/outputs'
import type { Serialized } from '@langchain/core/load/serializable'
import {
  MAX_MESSAGES,
  cut,
  recordQuietly,
  sha256,
  tokensFromUsage,
  type InferenceLedger,
  type Lane,
  type LedgerMessage,
  type Tokens,
} from './ledger.js'

/** Text out of a message whose content may be a string or a list of parts. */
export function messageText(message: BaseMessage): string {
  const { content } = message
  if (typeof content === 'string') return content
  return content
    .map((part) => {
      if (!part || typeof part !== 'object') return ''
      const p = part as Record<string, unknown>
      return typeof p.text === 'string' ? p.text : ''
    })
    .join('')
}

/** Reasoning the provider sent back as text, on whichever field it used. */
export function reasoningText(message: BaseMessage): string | undefined {
  const extra = (message.additional_kwargs ?? {}) as Record<string, unknown>
  if (typeof extra.reasoning_content === 'string' && extra.reasoning_content.trim()) {
    return extra.reasoning_content
  }
  const reasoning = extra.reasoning as { summary?: unknown } | string | undefined
  if (typeof reasoning === 'string' && reasoning.trim()) return reasoning
  if (reasoning && typeof reasoning === 'object' && Array.isArray(reasoning.summary)) {
    const text = reasoning.summary
      .map((part) => (part && typeof part === 'object' && 'text' in part ? String(part.text) : ''))
      .join('\n')
      .trim()
    if (text) return text
  }
  if (Array.isArray(message.content)) {
    const text = message.content
      .map((part) => {
        const p = part as Record<string, unknown>
        return (p.type === 'reasoning' || p.type === 'thinking') && typeof p.text === 'string'
          ? p.text
          : ''
      })
      .join('\n')
      .trim()
    if (text) return text
  }
  return undefined
}

function toolCallsOf(message: BaseMessage): Array<{ name: string; args: unknown }> {
  const calls = (message as { tool_calls?: Array<{ name: string; args: unknown }> }).tool_calls
  return Array.isArray(calls) ? calls : []
}

/** What an assistant turn said, or asked for: the text, then each tool call on its own line. */
export function outputText(message: BaseMessage): string {
  const lines = [messageText(message).trim()]
  for (const call of toolCallsOf(message)) {
    let args: string
    try {
      args = JSON.stringify(call.args)
    } catch {
      args = String(call.args)
    }
    lines.push(`→ ${call.name}(${args})`)
  }
  return lines.filter(Boolean).join('\n')
}

/**
 * The transcript the model was shown, without the system prompt. The first
 * user message is the task and always stays; after it, only the tail of the
 * tool trail, which is what this step actually answered.
 */
export function ledgerMessages(messages: BaseMessage[]): LedgerMessage[] {
  const rows: LedgerMessage[] = []
  for (const message of messages) {
    const type = message._getType()
    if (type === 'system') continue
    if (type === 'human') {
      rows.push({ role: 'user', text: cut(messageText(message)) })
    } else if (type === 'ai') {
      const tools = toolCallsOf(message).map((call) => call.name)
      rows.push({
        role: 'assistant',
        ...(tools.length ? { tool: tools.join(', ') } : {}),
        text: cut(outputText(message)),
      })
    } else if (type === 'tool') {
      const name = (message as { name?: string }).name
      rows.push({ role: 'tool', ...(name ? { tool: name } : {}), text: cut(messageText(message), 800) })
    }
  }
  if (rows.length <= MAX_MESSAGES) return rows
  const [first, ...rest] = rows
  return first ? [first, ...rest.slice(rest.length - (MAX_MESSAGES - 1))] : rows.slice(-MAX_MESSAGES)
}

/** A stable rendering of a message for hashing: type, content, tool calls, name. */
function canonical(message: BaseMessage): string {
  return JSON.stringify({
    type: message._getType(),
    content: message.content,
    name: (message as { name?: string }).name,
    tool_calls: toolCallsOf(message),
  })
}

export function promptDigest(messages: BaseMessage[]): string | undefined {
  const system = messages.filter((message) => message._getType() === 'system')
  return system.length ? sha256(...system.map(messageText)) : undefined
}

function tokensOf(message: BaseMessage): Tokens {
  const meta = (message.response_metadata ?? {}) as Record<string, unknown>
  const raw = tokensFromUsage(meta.usage)
  if (raw.total > 0) return raw
  const usage = (message as { usage_metadata?: unknown }).usage_metadata
  return tokensFromUsage(usage)
}

interface Pending {
  startedAt: number
  messages: BaseMessage[]
}

/**
 * The hook that writes a ledger row for every chat-model call a graph makes.
 *
 * LangChain reports the messages at start and the result at end, keyed by a
 * run id, so this keeps the one and waits for the other. A call that errors
 * leaves no row: there was no inference to prove.
 */
export class LedgerCallbacks extends BaseCallbackHandler {
  name = 'inference-ledger'
  private readonly pending = new Map<string, Pending>()

  constructor(
    private readonly options: {
      ledger: InferenceLedger
      lane: Lane
      source: string
      model: string
      log?: (message: string, error: unknown) => void
    },
  ) {
    super()
  }

  override async handleChatModelStart(
    _llm: Serialized,
    messages: BaseMessage[][],
    runId: string,
  ): Promise<void> {
    this.pending.set(runId, { startedAt: Date.now(), messages: messages[0] ?? [] })
  }

  override async handleLLMError(_error: unknown, runId: string): Promise<void> {
    this.pending.delete(runId)
  }

  override async handleLLMEnd(output: LLMResult, runId: string): Promise<void> {
    const started = this.pending.get(runId)
    this.pending.delete(runId)
    if (!started) return
    const generation = output.generations[0]?.[0] as { message?: BaseMessage } | undefined
    const message = generation?.message
    if (!message) return
    const meta = (message.response_metadata ?? {}) as Record<string, unknown>
    const reasoning = reasoningText(message)
    const responseId = typeof message.id === 'string' ? message.id : undefined
    const modelName = typeof meta.model_name === 'string' ? meta.model_name : this.options.model
    await recordQuietly(
      this.options.ledger,
      {
        lane: this.options.lane,
        source: this.options.source,
        model: modelName,
        input: ledgerMessages(started.messages),
        ...(reasoning ? { reasoning: cut(reasoning, 4_000) } : {}),
        output: cut(outputText(message), 4_000),
        tokens: tokensOf(message),
        ...(responseId ? { responseId } : {}),
        digest: sha256(...started.messages.map(canonical), canonical(message)),
        ...(() => {
          const digest = promptDigest(started.messages)
          return digest ? { promptDigest: digest } : {}
        })(),
        durationMs: Date.now() - started.startedAt,
      },
      this.options.log,
    )
  }
}
