import type { BaseMessage } from '@langchain/core/messages'
import { HumanMessage } from '@langchain/core/messages'
import type { StructuredToolInterface } from '@langchain/core/tools'
import type { BaseChatModel } from '@langchain/core/language_models/chat_models'
import { MemorySaver } from '@langchain/langgraph'
import { createReactAgent } from '@langchain/langgraph/prebuilt'
import { lastText, toolSteps } from '../agent/graph.js'
import type { AgentContext, ToolUse } from './types.js'

/**
 * One voice in the room. It is spoken to, may look things up, and answers.
 * The transcript between agents is only what they say; what each one
 * looked up stays in its own history.
 */
export interface BackroomsAgent {
  speak(input: { message: string; systemNote?: string }): Promise<{ text: string; tools: ToolUse[] }>
}

/**
 * A ReAct agent over a chat model with the assembled context as its
 * standing prompt and a private thread, so its tool results and its own
 * earlier lines are part of what it sees and nobody else's are.
 *
 * The step limit is the tool budget per turn: a model that keeps searching
 * instead of speaking is cut off and whatever text it managed is the turn.
 */
export class LangGraphBackroomsAgent implements BackroomsAgent {
  private readonly graph: ReturnType<typeof createReactAgent>
  private readonly threadId: string
  private seen = 0

  constructor(options: {
    llm: BaseChatModel
    context: AgentContext
    tools: StructuredToolInterface[]
    runId: string
    maxToolCalls: number
  }) {
    this.threadId = `backrooms:${options.runId}:${options.context.agent.id}`
    this.maxToolCalls = options.maxToolCalls
    this.graph = createReactAgent({
      llm: options.llm,
      tools: options.tools,
      prompt: renderSystem(options.context),
      checkpointer: new MemorySaver(),
    })
  }

  private readonly maxToolCalls: number

  async speak(input: { message: string; systemNote?: string }): Promise<{ text: string; tools: ToolUse[] }> {
    const content = input.systemNote ? `<SYSTEM MESSAGE> ${input.systemNote}\n\n${input.message}` : input.message
    let messages: BaseMessage[]
    try {
      const result = await this.graph.invoke(
        { messages: [new HumanMessage(content)] },
        {
          configurable: { thread_id: this.threadId },
          recursionLimit: 2 * this.maxToolCalls + 2,
        },
      )
      messages = result.messages as BaseMessage[]
    } catch (error) {
      // The recursion limit is the one error that still leaves a usable
      // history: read what was said before the cut-off.
      const state = await this.graph.getState({ configurable: { thread_id: this.threadId } })
      messages = ((state.values as { messages?: BaseMessage[] }).messages ?? [])
      if (messages.length <= this.seen) throw error
    }
    const fresh = messages.slice(this.seen)
    this.seen = messages.length
    return {
      text: lastText(fresh),
      tools: toolSteps(fresh).map((step) => ({ name: step.tool, input: step.input, summary: step.result ?? '' })),
    }
  }
}

/** The standing instructions, then every context block under its label. */
export function renderSystem(context: AgentContext): string {
  return [
    context.agent.system,
    '',
    ...context.blocks.flatMap((block) => [`## ${block.label}`, '', block.text, '']),
    `you speak as ${context.agent.marker}. answer with your line only: no marker, no name, nothing`,
    'for anyone but the room. never write the other side\'s lines.',
  ].join('\n')
}
