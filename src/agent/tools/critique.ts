import { tool } from '@langchain/core/tools'
import type { StructuredToolInterface } from '@langchain/core/tools'
import { z } from 'zod'
import { renderCritique, type Critic } from '../../critic/algo.js'

/**
 * The ranker's reader as a tool: a drafter hands over what it is about to
 * publish and gets back the For You arithmetic and the fixes. It publishes
 * nothing and holds nothing; a drafter that ignores it can still post. It
 * fails open too, in words, so a critic that is down costs one line of the
 * transcript and not the post.
 */
export function createCritiqueTools(critic: Critic): StructuredToolInterface[] {
  const critique = tool(
    async ({ kind, text, media, context }) => {
      try {
        const read = await critic.critique({
          kind,
          text,
          ...(media ? { media } : {}),
          ...(context ? { context } : {}),
        })
        return renderCritique(read)
      } catch (error) {
        return `The critic could not be reached (${error instanceof Error ? error.message : String(error)}). Use your own read of the conversation skill's pre-post check.`
      }
    },
    {
      name: 'x_critique',
      description:
        'Ask the ranker\'s reader what X\'s For You algorithm would do with a draft: the ' +
        'predicted reply, quote, share, and follow rates against the "not interested", mute ' +
        'and report risk, scored with the open-sourced weights, plus the first reply a ' +
        'stranger would type and concrete fixes. Call it on any standalone or quote post, ' +
        'caption, or clip script before publishing, and once more on the rewrite. It is a ' +
        'second opinion, not the author: take the fix, keep the voice. It publishes nothing.',
      schema: z.object({
        kind: z
          .enum(['post', 'quote', 'reply', 'caption', 'script', 'thread', 'other'])
          .default('post')
          .describe('What the draft is. A caption is the text going with a picture or clip.'),
        text: z.string().min(1).max(8_000).describe('The draft, exactly as it would be published.'),
        media: z
          .string()
          .max(600)
          .optional()
          .describe('What the attached picture or clip shows, in a sentence, when there is one.'),
        context: z
          .string()
          .max(2_000)
          .optional()
          .describe('The post being replied to or quoted, or the story a caption is about.'),
      }),
    },
  )
  return [critique]
}
