import { tool } from '@langchain/core/tools'
import type { StructuredToolInterface } from '@langchain/core/tools'
import { z } from 'zod'
import type { Memory, Person } from '../../types.js'

/** A handle to the person memory files under, or nothing for a stranger. */
export type FindPerson = (username: string) => Promise<Person | undefined>

export function createMemoryTools(
  memory: Memory,
  options: { findPerson?: FindPerson } = {},
): StructuredToolInterface[] {
  const recall = tool(
    async ({ query, about }) => {
      // Without `about`, a recall is a search over everything, and a quiet
      // person's history sinks under general context. With it, memory is
      // asked about that person specifically, the way the turn's own recall
      // already does for whoever is being answered.
      const person = about && options.findPerson ? await options.findPerson(about).catch(() => undefined) : undefined
      const context = await memory.recall(query, person ? { person } : {})
      const note = about && !person ? `(No one called ${about} is known; this is a general search.)\n` : ''
      return `${note}${context.trim() || 'No relevant memory was recalled.'}`
    },
    {
      name: 'recall_memory',
      description:
        'Search CortexDB long-term memory for prior interactions, product facts, tone, and ' +
        'past decisions. Call this before answering anything about the project or a user ' +
        'the bot has spoken to before. Pass `about` with an @handle to search what is known ' +
        'about, and what was said with, that particular person.',
      schema: z.object({
        query: z
          .string()
          .min(1)
          .describe('A natural-language question about what should be remembered.'),
        about: z
          .string()
          .regex(/^@?\w{1,15}$/)
          .optional()
          .describe('An X handle, e.g. "@someone", to focus the search on that person.'),
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
