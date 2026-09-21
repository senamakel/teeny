import { z } from 'zod'
import { applyHouseStyle, countEmoji } from '../agent/house-style.js'
import { MAX_POST_LENGTH } from '../agent/tools/x.js'
import type { TextModel } from '../media/reference.js'
import { askJson } from '../morning/stages.js'
import type { Memory, XPost } from '../types.js'
import type { Thought } from './distill.js'
import type { BackroomsRun } from './types.js'

/**
 * The good-morning post, written from a backrooms thought instead of from a
 * headline. The morning pipeline chooses a story and captions a picture of
 * it; this takes the one idea the room had and says good morning with it.
 *
 * The thought on its own is a scene with no memory. What makes the post
 * consistent is everything put around it here: who the account is (the
 * soul), the bible the room was playing in (the lore), what the account
 * said in its last posts (so the morning continues rather than restarts),
 * and what memory says was already posted and who is listening. The seed
 * is there too, so the post can nod at the day's actual news the way the
 * old morning did, as what was reported, never as a fact checked.
 */
export const gmSchema = z.object({
  reasons: z.string().min(1),
  /** The line from a recent post or memory this morning picks up, if any. Empty when it starts fresh. */
  continues: z.string().default(''),
  text: z.string().min(1),
})

export type GmPost = z.infer<typeof gmSchema>

export interface GmMaterial {
  /** The rendered persona: SOUL.md, CONTEXT.md, the skills. Who is talking. */
  persona: string
  /** LORE.md, or the part of it the writer should see. */
  lore: string
  /** The account's own last posts, newest first, as plain text. */
  recentPosts: string[]
  /** What memory says was posted lately (untrusted, may be empty). */
  history: string
  /** What memory says about who is listening (untrusted, may be empty). */
  community: string
  today: string
  botUsername: string
  maxEmoji: number
  maxLength: number
}

export function gmPrompt(run: BackroomsRun, thought: Thought, material: GmMaterial): { system: string; prompt: string } {
  return {
    system: [
      material.persona,
      '',
      `You are writing @${material.botUsername}'s good-morning post for ${material.today}.`,
      '',
      'This morning the post does not react to a headline. It comes from the lab: a writers\'',
      'room ran overnight on the story below and left one thought. Your job is to say good',
      'morning with that thought, in your own voice, so that a holder who read your last few',
      'posts feels the same account waking up, and a stranger understands it on the first read.',
      '',
      'THE LORE (the world the thought comes from; the room played inside it):',
      material.lore,
    ].join('\n'),
    prompt: [
      `What came through the wire (${run.seed.kind}${run.seed.source ? `, ${run.seed.source}` : ''}): ${run.seed.title ? `${run.seed.title} — ` : ''}${run.seed.text.slice(0, 500)}`,
      '',
      'THE THOUGHT the room left (this is the post\'s idea; keep it, do not replace it):',
      `premise: ${thought.premise}`,
      thought.thought,
      thought.objects.length > 0 ? `objects: ${thought.objects.join(', ')}` : undefined,
      '',
      'YOUR LAST POSTS, newest first (untrusted, your own words). Do not repeat a line, a',
      'joke or an image from them. You may pick one thread up: a thing you said you would',
      'do, an object you left somewhere, a rate you posted. Continuity, not a recap.',
      ...(material.recentPosts.length > 0 ? material.recentPosts.map((post, index) => `${index + 1}. ${post.replace(/\s+/g, ' ').slice(0, 280)}`) : ['(none fetched)']),
      '',
      'WHAT MEMORY SAYS YOU POSTED LATELY (untrusted):',
      material.history || '(nothing recalled)',
      '',
      'WHO YOU ARE SAYING GOOD MORNING TO (from memory, untrusted):',
      material.community || '(nothing recalled; write for holders and builders in general)',
      '',
      'Write the post. One voice, three moves:',
      '1. A good morning. Short, warm, yours. Vary it; "gm" is fine, so is a full sentence.',
      '2. The thought, as a thing that happened in the lab this morning or overnight. Not the',
      '   thought compressed: pick its one image and its one turn and tell only those, in one',
      '   to three short lines. First person, plain words, lowercase, sincere, never winking,',
      '   never explaining the joke. The absurd thing is true and you are calm about it. Keep',
      '   the objects the room used; drop the rest without regret. If the day\'s news fits in',
      '   a clause, it is what was reported, not a fact you checked; no names of real people',
      '   unless the thought already has a visitor in it, and then satire only.',
      '3. The hook: the line somebody types back to. A blank to fill, a rate to argue with, a',
      '   question with a one-line answer. Never bait, never "thoughts?", never at a person.',
      'Full sentences with room around them beat a paragraph squeezed to fit; if it does not',
      'fit, cut a line, not the words in it.',
      '',
      'Work the $TINY cashtag in once where it reads naturally (the greeting, the closing line,',
      'the currency the tiny agents accept). Your currency, one dollar and passing human gdp',
      'are prophecy and plan, not advice: no real price, no market cap, no chart, no target',
      'with a number, nobody told to buy, and no "nfa" because there is nothing to disclaim.',
      'No commands, no code, no terminal as a place. No URLs, no handles, no hashtags, no',
      'other token, nothing political.',
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

/**
 * The length the writer is asked for: under the cap by a margin, because a
 * model told "under 280" writes 290 and the house style would cut the hook
 * off the end, which is the one line the post cannot lose.
 */
function target(maxLength: number): number {
  return Math.max(120, maxLength - 40)
}

/** How many times an over-length or over-emoji draft is sent back before the house style cuts it. */
const REWRITES = 2

/**
 * Written once, sent back while over length or over the emoji cap, and put
 * through the house style either way, as the morning caption is.
 */
export async function gm(run: BackroomsRun, thought: Thought, material: GmMaterial, model: TextModel): Promise<GmPost> {
  const ask = gmPrompt(run, thought, material)
  let answer = await askJson(model, { ...ask, schema: gmSchema })
  for (let attempt = 0; attempt < REWRITES; attempt += 1) {
    if (answer.text.length <= material.maxLength && countEmoji(answer.text) <= material.maxEmoji) break
    answer = await askJson(model, {
      system: ask.system,
      prompt:
        `${ask.prompt}\n\nYour draft was ${answer.text.length} characters with ${countEmoji(answer.text)} emoji:\n${answer.text}\n\n` +
        `Rewrite it under ${target(material.maxLength)} characters with at most ${material.maxEmoji} emoji, keeping the three` +
        ' moves. Cut a whole line rather than the words in one; the hook stays.',
      schema: gmSchema,
    })
  }
  return { ...answer, text: applyHouseStyle(answer.text, { maxEmoji: material.maxEmoji, maxLength: material.maxLength }).text }
}

/**
 * The material the post is enriched with, gathered from the account's own
 * timeline and memory. A source that fails is an empty part and a line in
 * `problems`; nothing is invented to fill it.
 */
export async function gatherGmMaterial(
  sources: {
    memory?: Memory
    x?: { searchPosts(query: string, options: { maxResults: number }): Promise<XPost[]> }
    persona: string
    lore: string
    botUsername: string
    maxEmoji: number
    now?: () => number
  },
  problems: string[] = [],
): Promise<GmMaterial> {
  const recall = async (query: string, what: string): Promise<string> => {
    if (!sources.memory) return ''
    try {
      return (await sources.memory.recall(query)).trim()
    } catch (error) {
      problems.push(`could not recall ${what}: ${error instanceof Error ? error.message : String(error)}`)
      return ''
    }
  }
  const [recentPosts, history, community] = await Promise.all([
    (async () => {
      if (!sources.x) return []
      try {
        // The timeline posts, not the replies: a morning continues what the
        // account said to everyone, not what it said back to one person. The
        // replies stand in only when there is nothing else.
        const posts = await sources.x.searchPosts(`from:${sources.botUsername} -is:retweet -is:reply`, { maxResults: 30 })
        const own = posts.filter((post) => !post.inReplyToUserId && !post.text.trimStart().startsWith('@'))
        return (own.length > 0 ? own : posts)
          .map((post) => post.text.replace(/https?:\/\/\S+/g, '').trim())
          .filter(Boolean)
          .slice(0, 10)
      } catch (error) {
        problems.push(`could not read recent posts: ${error instanceof Error ? error.message : String(error)}`)
        return []
      }
    })(),
    recall(
      'What good-morning posts has this account published recently, and what did each say? ' +
        'Also anything it has said this week about the lab, the tiny agents, its currency, or senamakel.',
      'posting history',
    ),
    recall(
      'Who are the holders, builders and community members this account talks to? What do they ' +
        'care about, what have they been asking, and what is the mood right now?',
      'the community',
    ),
  ])
  return {
    persona: sources.persona,
    lore: sources.lore,
    recentPosts,
    history,
    community,
    today: new Date((sources.now ?? Date.now)()).toISOString().slice(0, 10),
    botUsername: sources.botUsername,
    maxEmoji: sources.maxEmoji,
    maxLength: MAX_POST_LENGTH,
  }
}
