import { HumanMessage, isAIMessage, type BaseMessage } from '@langchain/core/messages'
import type { StructuredToolInterface } from '@langchain/core/tools'
import { ChatOpenAI } from '@langchain/openai'
import { MemorySaver } from '@langchain/langgraph'
import { createReactAgent } from '@langchain/langgraph/prebuilt'
import { createAirdropTools, type AirdropContext, type AirdropToolOptions } from './tools/airdrop.js'
import { createExaClient, createExaTools } from './tools/exa.js'
import { createGiphyTools } from './tools/giphy.js'
import { createMemoryTools } from './tools/memory.js'
import { createXTools } from './tools/x.js'
import { systemPrompt } from './prompt.js'
import type { Persona } from './soul.js'
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
   * The wallet, its ledger and its ceilings. Absent, there is no signer in the
   * tool set at all and no amount of prompting can conjure one.
   */
  airdrop?: Omit<AirdropToolOptions, 'context'>
}

export function createChatModel(options: LlmOptions): ChatOpenAI {
  return new ChatOpenAI({
    model: options.model,
    apiKey: options.apiKey,
    temperature: options.temperature ?? 0.3,
    configuration: { baseURL: options.baseUrl },
  })
}

/**
 * The tools for one turn.
 *
 * `airdropContext` is what binds the signer to the post being answered, and the
 * airdrop tool is only built when it is supplied. That is the whole reason this
 * takes a second argument: a tool set built once and reused across turns would
 * let one turn pay out against another turn's post, and the operator endpoint,
 * which answers a task rather than a post, gets no signer by construction.
 */
export function buildTools(
  options: AgentOptions,
  airdropContext?: () => AirdropContext,
): StructuredToolInterface[] {
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
    ...(options.airdrop && airdropContext
      ? createAirdropTools({ ...options.airdrop, context: airdropContext })
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

function toolSteps(messages: BaseMessage[]): AgentRunResult['steps'] {
  return messages.flatMap((message) =>
    isAIMessage(message)
      ? (message.tool_calls ?? []).map((call) => ({ tool: call.name, input: call.args }))
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
