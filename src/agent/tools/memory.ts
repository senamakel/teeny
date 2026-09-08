import { tool } from '@langchain/core/tools'
import type { StructuredToolInterface } from '@langchain/core/tools'
import { z } from 'zod'
import type { Memory } from '../../types.js'

export function createMemoryTools(memory: Memory): StructuredToolInterface[] {
  const recall = tool(
    async ({ query }) => {
      const context = await memory.recall(query)
      return context.trim() || 'No relevant memory was recalled.'
    },
    {
      name: 'recall_memory',
      description:
        'Search CortexDB long-term memory for prior interactions, product facts, tone, and ' +
        'past decisions. Call this before answering anything about the project or a user ' +
        'the bot has spoken to before.',
      schema: z.object({
        query: z
          .string()
          .min(1)
          .describe('A natural-language question about what should be remembered.'),
      }),
    },
  )

  const remember = tool(
    async ({ note, labels }) => {
      const id = `note:${Date.now()}:${Math.random().toString(36).slice(2, 10)}`
      await memory.captureActivity('agent_note', id, { note, labels: labels ?? [] })
      return `Stored as ${id}.`
    },
    {
      name: 'remember',
      description:
        'Write a durable note to CortexDB memory. Use for facts worth recalling in future ' +
        'turns: decisions taken, promises made, user preferences, correction of a mistake.',
      schema: z.object({
        note: z.string().min(1).max(2_000),
        labels: z.array(z.string()).max(8).optional(),
      }),
    },
  )

  return [recall, remember]
}
