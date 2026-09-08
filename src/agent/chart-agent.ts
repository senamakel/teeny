import { HumanMessage, isAIMessage, type BaseMessage } from '@langchain/core/messages'
import type { StructuredToolInterface } from '@langchain/core/tools'
import { createReactAgent } from '@langchain/langgraph/prebuilt'
import { createChatModel, type LlmOptions } from './graph.js'
import { chartPrompt } from './chart-prompt.js'
import { createExaClient, createExaTools } from './tools/exa.js'
import { createMarketTools } from './tools/market.js'
import { createMemoryTools } from './tools/memory.js'
import { createXTools } from './tools/x.js'
import type { Persona } from './soul.js'
import type { ChartRenderer } from '../chart/renderer.js'
import type { TokenMarket } from '../market/market.js'
import type { Agent, AgentRunResult, Memory, XTools } from '../types.js'

export interface ChartAgentOptions {
  memory: Memory
  market: TokenMarket
  charts: ChartRenderer
  x: XTools
  address: string
  symbol: string
  botUsername: string
  dryRun: boolean
  llm: LlmOptions
  persona?: Persona
  exaApiKey?: string
  maxSteps?: number
  maxEmoji?: number
  searchLimit?: number
  watermark?: string
  outputDir?: string
  /** Off, the agent researches and drafts but `post_chart` refuses. */
  allowPosting?: boolean
}

/**
 * The chart poster.
 *
 * A separate agent from the reply drafter, with a separate prompt and a
 * separate set of tools, because they are separate jobs: one answers a person
 * and the other publishes an analysis to everybody. It shares the persona and
 * the same CortexDB memory, so the voice is one voice and yesterday's post is
 * something it can actually remember making.
 *
 * It reads X but cannot reply, and publishes only through `post_chart`, which
 * takes one post and one image.
 */
export class ChartAgent implements Agent {
  private readonly graph: ReturnType<typeof createReactAgent>
  private readonly maxSteps: number

  constructor(options: ChartAgentOptions) {
    this.maxSteps = options.maxSteps ?? 20
    this.graph = createReactAgent({
      llm: createChatModel(options.llm),
      tools: this.buildTools(options),
      prompt: chartPrompt({
        botUsername: options.botUsername,
        symbol: options.symbol,
        dryRun: options.dryRun,
        ...(options.persona ? { persona: options.persona } : {}),
      }),
    })
  }

  private buildTools(options: ChartAgentOptions): StructuredToolInterface[] {
    return [
      ...createMemoryTools(options.memory),
      ...createMarketTools({
        market: options.market,
        address: options.address,
        symbol: options.symbol,
        x: options.x,
        charts: options.charts,
        dryRun: options.dryRun,
        ...(options.maxEmoji === undefined ? {} : { maxEmoji: options.maxEmoji }),
        ...(options.watermark ? { watermark: options.watermark } : {}),
        ...(options.outputDir ? { outputDir: options.outputDir } : {}),
        ...(options.allowPosting === false ? { allowPosting: false } : {}),
      }),
      // Reading X only. Publishing goes through post_chart, which is the tool
      // that knows how to attach the image and how to count the emoji.
      ...createXTools(options.x, {
        dryRun: options.dryRun,
        allowPosting: false,
        ...(options.searchLimit === undefined ? {} : { searchLimit: options.searchLimit }),
      }),
      ...(options.exaApiKey ? createExaTools(createExaClient(options.exaApiKey)) : []),
    ]
  }

  async run(input: { task: string; threadId?: string }): Promise<AgentRunResult> {
    const result = await this.graph.invoke(
      { messages: [new HumanMessage(input.task)] },
      { recursionLimit: this.maxSteps * 2 },
    )
    const messages = result.messages as BaseMessage[]
    return {
      output: lastText(messages),
      steps: messages.flatMap((message) =>
        isAIMessage(message)
          ? (message.tool_calls ?? []).map((call) => ({ tool: call.name, input: call.args }))
          : [],
      ),
    }
  }
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
