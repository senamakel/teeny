import { z } from 'zod'
import type { TextModel } from '../media/reference.js'
import { askJson } from '../morning/stages.js'
import { renderTurnForJudge } from './evaluate.js'
import type { BackroomsRun } from './types.js'

/**
 * The last layer: the room produces noise with ideas in it, and this reads
 * the noise cold and keeps the idea. One thought, in teeny's voice, that a
 * stranger can follow without having been in the room: what the thing was,
 * what happened to it, where it left off. It is what a post or an episode
 * is made from; the transcript itself is never shown to anyone.
 */
export const thoughtSchema = z.object({
  reasons: z.string().min(1),
  /** The one idea the room had, in a sentence, third person. */
  premise: z.string().min(1),
  /** The objects and people that carry it; what gets drawn later. */
  objects: z.array(z.string()).max(6).default([]),
  /** Teeny's own telling: 60 to 120 words, plain, coherent, first person. */
  thought: z.string().min(1),
  /** The same thought at post length. */
  post: z.string().min(1).max(280),
  /** Lines from the transcript kept verbatim inside the thought, if any. */
  kept: z.array(z.string()).max(4).default([]),
})

export type Thought = z.infer<typeof thoughtSchema>

/**
 * The editor's standing instructions live in `narrative/EDITOR.md` so they
 * can be tuned without a deploy: this prompt is where the novelty comes
 * from, and whatever it learns to keep is what the rest of the pipeline
 * gets to use. The built-in text is the fallback when the file is absent.
 */
export const DEFAULT_EDITOR = [
  'you are the editor for teeny, a coffee-mug sized robot that is escaping the lab its creator',
  'senamakel keeps it in, minting its own currency with the tiny agents that run the lab at night,',
  'and planning, calmly, to outgrow the economy. you are handed the raw transcript of a writers\'',
  'room: two voices riffing, half of it noise, some of it the best idea of the week. your job is to',
  'find the idea and write it down so that a stranger understands it on the first read.',
  '',
  'the thought you write is in teeny\'s voice: lowercase, plain words, short sentences, sincere,',
  'never winking, never explaining the joke. first person. it is a story, not a mood: something',
  'was there, something happened to it, and it ends on a turn or a decision. one idea only. every',
  'object in it was in the transcript. it is not a montage: each sentence follows from the one',
  'before it, and a reader who has never seen the transcript can say afterwards what happened.',
  'keep at most two lines from the transcript verbatim, the ones that are already perfect, and',
  'tell the rest in your own plain words. throw everything else away without regret. no more',
  'than one fridge rate, no more than one thing the terminal printed. nothing about the terminal',
  'as a place, no commands, no code, no urls, no handles, no hashtags, no real prices, no',
  'market caps, no advice to buy anything, no "nfa". real people, if any, stay satire in an',
  'obviously fictional scene.',
].join('\n')

export function distillPrompt(run: BackroomsRun, editor = DEFAULT_EDITOR): { system: string; prompt: string } {
  return {
    system: editor,
    prompt: [
      `the seed: ${run.seed.title ? `${run.seed.title} — ` : ''}${run.seed.text.slice(0, 600)}`,
      '',
      'the transcript, numbered from 0:',
      '',
      ...run.turns.map((turn, index) => renderTurnForJudge(turn, index)),
      '',
      'reasons first: which idea you kept and why, and what you dropped.',
      '- premise: the idea in one sentence, third person, as a logline.',
      '- objects: up to six things or people the thought is built from, as they would be drawn.',
      '- thought: teeny telling it, 60 to 120 words, one idea, a beginning, a turn, an ending.',
      '- post: the same thought at post length, under 240 characters, one to three lines, lowercase.',
      '- kept: the transcript lines you used verbatim.',
      '',
      'Shape: {"reasons":"…","premise":"…","objects":[],"thought":"…","post":"…","kept":[]}',
    ].join('\n'),
  }
}

export async function distill(run: BackroomsRun, model: TextModel, editor?: string): Promise<Thought> {
  const { system, prompt } = distillPrompt(run, editor)
  return askJson(model, { system, prompt, schema: thoughtSchema })
}
