import { HumanMessage, isAIMessage, isToolMessage, type BaseMessage } from '@langchain/core/messages'
import type { StructuredToolInterface } from '@langchain/core/tools'
import { ChatOpenAI } from '@langchain/openai'
import { MemorySaver } from '@langchain/langgraph'
import { createReactAgent } from '@langchain/langgraph/prebuilt'
import { createExaClient, createExaTools } from './tools/exa.js'
import { createGiphyTools } from './tools/giphy.js'
import { createMarketTools } from './tools/market.js'
import { createMemoryTools } from './tools/memory.js'
import { createXTools } from './tools/x.js'
import { systemPrompt } from './prompt.js'
import { summariseResult } from '../journal/journal.js'
import type { Persona } from './soul.js'
import type { GlobalMarketClient } from '../market/global.js'
import type { TokenMarket } from '../market/market.js'
import type { Agent, AgentRunResult, GifSource, Memory, XTools } from '../types.js'

export interface LlmOptions {
  baseUrl: string
  apiKey: string
  model: string
  temperature?: number
}

export interface AgentOptions {
  memory: Memory
  x: XTools
  botUsername: string
  dryRun: boolean
  llm: LlmOptions
  exaApiKey?: string
  maxSteps?: number
  /** Set false to build a research-only agent that cannot publish. */
  allowPosting?: boolean
  /** SOUL.md and the active skills, prepended to the system prompt. */
  persona?: Persona
  /** Hard cap on emoji per published post. Defaults to 1. */
  maxEmoji?: number
  /** The account's GIF channel. Without it the agent posts text only. */
  gifs?: { source: GifSource; channel: string }
  /** Cap on posts pulled per search by the agent's own X tools. */
  searchLimit?: number
  /** The account's own token contracts, recognised before drafting. */
  ownTokenAddresses?: string[]
  /**
   * Read-only market data for the account's own token.
   *
   * Without it the drafter cannot look at its own chart, which is not a
   * hypothetical gap: asked "much higher for tiny?", the account answered "I'm
   * just a mascot who doesn't read charts" -- hours after publishing one. That
   * was an accurate description of the tools it had. It gets no renderer and no
   * publish tool here, so it can read the market and still cannot broadcast.
   */
  market?: {
    market: TokenMarket
    address: string
    symbol?: string
    global?: GlobalMarketClient
  }
}

export function createChatModel(options: LlmOptions): ChatOpenAI {
  return new ChatOpenAI({
    model: options.model,
    apiKey: options.apiKey,
    temperature: options.temperature ?? 0.3,
    configuration: { baseURL: options.baseUrl },
  })
}

export function buildTools(options: AgentOptions): StructuredToolInterface[] {
  return [
    ...createMemoryTools(options.memory),
    ...createXTools(options.x, {
      dryRun: options.dryRun,
      ...(options.allowPosting === false ? { allowPosting: false } : {}),
      ...(options.maxEmoji === undefined ? {} : { maxEmoji: options.maxEmoji }),
      ...(options.gifs ? { gifs: options.gifs } : {}),
      ...(options.searchLimit === undefined ? {} : { searchLimit: options.searchLimit }),
    }),
    ...(options.exaApiKey ? createExaTools(createExaClient(options.exaApiKey)) : []),
    ...(options.gifs ? createGiphyTools(options.gifs.source, options.gifs.channel) : []),
    ...(options.market
      ? createMarketTools({
          market: options.market.market,
          address: options.market.address,
          ...(options.market.symbol ? { symbol: options.market.symbol } : {}),
          ...(options.market.global ? { global: options.market.global } : {}),
          x: options.x,
          dryRun: options.dryRun,
          allowPosting: false,
        })
      : []),
  ]
}

function lastText(messages: BaseMessage[]): string {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index]
    if (!message || !isAIMessage(message)) continue
    const { content } = message
    const text =
      typeof content === 'string'
        ? content
        : content
            .map((part) => ('text' in part && typeof part.text === 'string' ? part.text : ''))
            .join('')
    if (text.trim()) return text.trim()
  }
  return ''
}

/**
 * Every tool call the run made, in order, each paired with a line of what came
 * back. The result is found by tool_call_id rather than by position because
 * a model may issue several calls in one turn and the replies can land in
 * any order.
 */
export function toolSteps(messages: BaseMessage[]): AgentRunResult['steps'] {
  const results = new Map<string, string>()
  for (const message of messages) {
    if (isToolMessage(message) && message.tool_call_id) {
      results.set(message.tool_call_id, summariseResult(message.content))
    }
  }
  return messages.flatMap((message) =>
    isAIMessage(message)
      ? (message.tool_calls ?? []).map((call) => {
          const result = call.id ? results.get(call.id) : undefined
          return { tool: call.name, input: call.args, ...(result !== undefined ? { result } : {}) }
        })
      : [],
  )
}

/** A LangGraph ReAct agent wired to CortexDB memory, Exa research, and the X API. */
export class LangGraphAgent implements Agent {
  private readonly graph: ReturnType<typeof createReactAgent>
  private readonly maxSteps: number

  constructor(options: AgentOptions) {
    this.maxSteps = options.maxSteps ?? 24
    this.graph = createReactAgent({
      llm: createChatModel(options.llm),
      tools: buildTools(options),
      prompt: systemPrompt({
        botUsername: options.botUsername,
        dryRun: options.dryRun,
        ...(options.persona ? { persona: options.persona } : {}),
      }),
      checkpointer: new MemorySaver(),
    })
  }

  async run(input: { task: string; threadId?: string }): Promise<AgentRunResult> {
    const result = await this.graph.invoke(
      { messages: [new HumanMessage(input.task)] },
      {
        configurable: { thread_id: input.threadId ?? `task:${Date.now()}` },
        recursionLimit: this.maxSteps * 2,
      },
    )
    const messages = result.messages as BaseMessage[]
    return { output: lastText(messages), steps: toolSteps(messages) }
  }
}
