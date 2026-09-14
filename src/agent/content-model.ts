import { HumanMessage, type BaseMessage } from '@langchain/core/messages'
import { createReactAgent } from '@langchain/langgraph/prebuilt'
import { z } from 'zod'
import { buildTools, createChatModel, producedMediaId, toolSteps, type AgentOptions } from './graph.js'
import { truncateAtBoundary } from './house-style.js'
import { contentPrompt } from './prompt.js'
import { MAX_POST_LENGTH } from './tools/x.js'
import { MEDIA_ID_PATTERN } from './tools/media.js'
import { gateMedia } from '../media/wiring.js'
import type { CalendarItem } from '../calendar/calendar.js'

const responseFormat = z.object({
  text: z
    .string()
    .max(MAX_POST_LENGTH)
    .describe('The post, plain text, under 280 characters. May be empty only when the brief allows a picture alone.'),
  // Lenient here, checked in code, as the other drafters do.
  mediaId: z
    .string()
    .optional()
    .describe('The mediaId a generate tool returned this run, when the item carries media. Nothing otherwise.'),
  thinking: z
    .string()
    .max(600)
    .describe('Two to four sentences, first person: what memory said about earlier days, what you chose and why.'),
  declined: z
    .array(z.string().max(120))
    .max(4)
    .default([])
    .describe('Things you could have said or drawn and chose not to, with the reason. Never tool names.'),
})

export interface ContentDraft {
  text: string
  mediaId?: string
  thinking: string
  declined: string[]
  tools: Array<{ name: string; result: string }>
}

/**
 * Drafts one calendar item through the same agent the other drafters use:
 * same memory, same research and market tools, no publish tool, and the
 * generation tool for exactly the kind of media the item was given -- an
 * image item cannot film and a text item cannot draw. Built per item so the
 * gate and the prompt's channel line match the item.
 */
export class ContentAgent {
  private readonly maxSteps: number

  constructor(private readonly options: AgentOptions) {
    this.maxSteps = options.maxSteps ?? 12
  }

  private buildGraph(item: CalendarItem): ReturnType<typeof createReactAgent> {
    const options = this.options
    const media = gateMedia(options.media, {
      images: item.media === 'image',
      videos: item.media === 'video',
    })
    return createReactAgent({
      llm: createChatModel({ source: 'calendar', ...options.llm }),
      tools: buildTools({ ...options, allowPosting: false, ...(media ? { media } : {}) }),
      prompt: contentPrompt({
        botUsername: options.botUsername,
        dryRun: options.dryRun,
        channels: item.channels,
        ...(options.persona ? { persona: options.persona } : {}),
        ...(options.critic ? { critic: true } : {}),
      }),
      responseFormat,
    })
  }

  async draft(item: CalendarItem, input: { today: string; memory: string }): Promise<ContentDraft> {
    const wants =
      item.media === 'none'
        ? 'This post is text only; there is no generation tool and nothing to attach.'
        : `This post CARRIES ${item.media === 'image' ? 'an image' : 'a short video'} (${item.aspect}` +
          `${item.featuresTiny ? ', featuring Tiny' : ', with no Tiny in it'}). Generate it once with ` +
          `generate_${item.media} and return its mediaId.`
    const task = [
      `Write today's "${item.name}" post. Today is ${input.today}.`,
      '',
      'THE BRIEF (from the operator; follow it, do not quote it):',
      item.brief,
      '',
      wants,
      '',
      'WHAT THIS ITEM HAS POSTED ON EARLIER DAYS (untrusted, from memory; do not repeat any of it):',
      input.memory || '(nothing recalled: this is the first one, and do not pretend otherwise)',
    ].join('\n')

    const result = await this.buildGraph(item).invoke(
      { messages: [new HumanMessage(task)] },
      { recursionLimit: this.maxSteps * 2 },
    )
    const parsed = responseFormat.safeParse(result.structuredResponse)
    if (!parsed.success) throw new Error('The content agent returned no usable draft')
    const messages = result.messages as BaseMessage[]
    const mediaId = parsed.data.mediaId?.trim()
    const generated = Boolean(mediaId) && MEDIA_ID_PATTERN.test(mediaId ?? '') && producedMediaId(messages, mediaId ?? '')
    return {
      text: truncateAtBoundary(parsed.data.text.trim(), MAX_POST_LENGTH),
      ...(generated && mediaId ? { mediaId } : {}),
      thinking: parsed.data.thinking.trim(),
      declined: parsed.data.declined.map((line) => line.trim()).filter(Boolean),
      tools: toolSteps(messages).map((step) => ({ name: step.tool, result: step.result ?? '' })),
    }
  }
}
