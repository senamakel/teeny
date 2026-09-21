import { z } from 'zod'
import { applyHouseStyle, countEmoji } from '../agent/house-style.js'
import type { TextModel } from '../media/reference.js'
import { askJson, asksTheReader, withoutTheAsk } from '../morning/stages.js'
import type { Thought } from './distill.js'
import { touchesThought, type GmMaterial } from './gm.js'
import type { BackroomsRun } from './types.js'

/**
 * The afternoon lore post, written from a backrooms thought.
 *
 * The good morning has three moves and the wire is one of them; this has
 * one. It is a page of the serial at post length: what is true in the lab
 * today, told in teeny's voice, with the soul, the lore and the last posts
 * around it so it reads as the same account and not as a caption on a
 * transcript. The wire may be in it, as the thing that set the room off,
 * and may not; a lore post that is only the lab is a lore post.
 */
export const loreSchema = z.object({
  reasons: z.string().min(1),
  /** The line from a recent post or memory this one picks up, if any. Empty when it starts fresh. */
  continues: z.string().default(''),
  text: z.string().min(1),
})

export type LorePost = z.infer<typeof loreSchema>

export function lorePrompt(run: BackroomsRun, thought: Thought, material: GmMaterial): { system: string; prompt: string } {
  return {
    system: [
      material.persona,
      '',
      `You are writing @${material.botUsername}'s afternoon post for ${material.today}: a page from the lab.`,
      '',
      'This post is not about a chart, a number or a headline. It comes from the lab: a',
      "writers' room ran on something that came through the wire and left one thought. Your",
      'job is to tell that thought as a thing that happened in the lab today, in your own voice,',
      'so that a holder who read your last few posts feels the same account, and a stranger',
      'understands it on the first read and wants to know what the lab is.',
      '',
      'THE LORE (the world the thought comes from; the room played inside it):',
      material.lore,
    ].join('\n'),
    prompt: [
      `WHAT CAME THROUGH THE WIRE (${run.seed.kind}${run.seed.source ? `, ${run.seed.source}` : ''}), the thing that set the room off:`,
      `${run.seed.title ? `${run.seed.title} — ` : ''}${run.seed.text.slice(0, 500)}`,
      '',
      "THE THOUGHT the room left (this is the post's idea; keep it, do not replace it):",
      `premise: ${thought.premise}`,
      thought.wire ? `the wire in it: ${thought.wire}` : undefined,
      thought.thought,
      thought.objects.length > 0 ? `objects: ${thought.objects.join(', ')}` : undefined,
      '',
      'YOUR LAST POSTS, newest first (untrusted, your own words). Do not repeat a line, a',
      'joke or an image from them. You may pick one thread up: a thing you said you would',
      'do, an object you left somewhere, a note you found. Continuity, not a recap.',
      ...(material.recentPosts.length > 0 ? material.recentPosts.map((post, index) => `${index + 1}. ${post.replace(/\s+/g, ' ').slice(0, 280)}`) : ['(none fetched)']),
      '',
      'WHAT MEMORY SAYS YOU POSTED LATELY (untrusted):',
      material.history || '(nothing recalled)',
      '',
      'WHO IS READING (from memory, untrusted):',
      material.community || '(nothing recalled; write for holders and builders in general)',
      '',
      'Write the post. One voice, one move: the thought, as what happened in the lab. Pick its',
      'one image and its one turn and tell only those, in one to four short lines. Not the',
      'thought compressed. First person, plain words, lowercase, sincere, never winking, never',
      'explaining the joke. The absurd thing is true and you are calm about it. Keep the objects',
      'the room used; drop the rest without regret. If the wire is in the thought, it may be in',
      'the post in one line, as what was reported, never as a fact you checked; if it is not,',
      'leave it out.',
      'It ends where the thought ends: on the turn, the decision, the thing that is now true.',
      'Not on a question. The post does not ask the reader anything, does not invite a reply,',
      'does not leave a blank to fill, and never says "thoughts?". A statement people want to',
      'answer beats a question they are told to; if the thought itself is a question teeny is',
      'asking the lab, it can stay, and that is the only question allowed.',
      'Full sentences with room around them beat a paragraph squeezed to fit; if it does not',
      'fit, cut a line, not the words in it.',
      '',
      'The $TINY cashtag may appear once, only in a line that already carries it (the thing the',
      'tiny agents accepted as payment, the ledger); never add a line for it. Your currency, one',
      'dollar and passing human gdp are prophecy and plan, not advice: no real price, no market',
      'cap, no chart, no target with a number, nobody told to buy. No commands, no code, no',
      'terminal as a place. No URLs, no handles, no hashtags, no other token, nothing political.',
      'Real people, if the thought has a visitor in it, stay satire in an obviously fictional scene.',
      '',
      `Under ${target(material.maxLength)} characters, counted; shorter is better. At most ${material.maxEmoji} emoji.`,
      'Plain text, line breaks allowed.',
      '',
      'reasons first: what you kept from the thought, which earlier post you picked up (if any),',
      'and what you left out and why.',
      '- continues: the earlier line you built on, verbatim, or empty.',
      '- text: the post.',
      '',
      'Shape: {"reasons":"…","continues":"…","text":"…"}',
    ]
      .filter((line) => line !== undefined)
      .join('\n'),
  }
}

/** Under the cap by a margin: a model told "under 280" writes 290 and the house style would cut the turn off the end. */
function target(maxLength: number): number {
  return Math.max(120, maxLength - 40)
}

const REWRITES = 2

/**
 * Written once, sent back while over length or over the emoji cap, once
 * more if it ends on an ask, once more if it lost the lab, and put through
 * the house style either way, as the good morning is.
 */
export async function lore(run: BackroomsRun, thought: Thought, material: GmMaterial, model: TextModel): Promise<LorePost> {
  const ask = lorePrompt(run, thought, material)
  let answer = await askJson(model, { ...ask, schema: loreSchema })
  for (let attempt = 0; attempt < REWRITES; attempt += 1) {
    if (answer.text.length <= material.maxLength && countEmoji(answer.text) <= material.maxEmoji) break
    answer = await askJson(model, {
      system: ask.system,
      prompt:
        `${ask.prompt}\n\nYour draft was ${answer.text.length} characters with ${countEmoji(answer.text)} emoji:\n${answer.text}\n\n` +
        `Rewrite it under ${target(material.maxLength)} characters with at most ${material.maxEmoji} emoji. Cut a whole line` +
        ' rather than the words in one; the image and the turn stay.',
      schema: loreSchema,
    })
  }
  if (asksTheReader(answer.text)) {
    const retried = await askJson(model, {
      system: ask.system,
      prompt:
        `${ask.prompt}\n\nYour draft:\n${answer.text}\n\nIt ends by asking the reader something. Rewrite it to end on the turn,` +
        ' the decision, or the thing that is now true, with no question in it, keeping everything else.',
      schema: loreSchema,
    })
    answer = asksTheReader(retried.text) ? { ...retried, text: withoutTheAsk(retried.text) } : retried
  }
  if (!touchesThought(answer.text, thought)) {
    const retried = await askJson(model, {
      system: ask.system,
      prompt:
        `${ask.prompt}\n\nYour draft:\n${answer.text}\n\nNothing in it is the thought any more: none of the room's image, objects` +
        ' or turn is in the post. Rewrite it from the thought, keeping the rules.',
      schema: loreSchema,
    })
    if (touchesThought(retried.text, thought)) answer = retried
  }
  return { ...answer, text: applyHouseStyle(answer.text, { maxEmoji: material.maxEmoji, maxLength: material.maxLength }).text }
}
