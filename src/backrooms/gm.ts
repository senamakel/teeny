import { z } from 'zod'
import { applyHouseStyle, countEmoji } from '../agent/house-style.js'
import { MAX_POST_LENGTH } from '../agent/tools/x.js'
import type { TextModel } from '../media/reference.js'
import { renderCritique, type Critique } from '../critic/algo.js'
import { askJson } from '../morning/stages.js'
import type { Memory, XPost } from '../types.js'
import type { Thought } from './distill.js'
import { keywords } from './sources/text.js'
import type { BackroomsRun, Seed } from './types.js'

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
 *
 * The seed is not optional in the post. A room drifts to its habits and an
 * editor keeps the best idea, and by the time the post is written the
 * day's news can be gone entirely; then the morning reads like any other
 * morning. So the news is one of the post's moves, the writer names the
 * line that carries it, and a post that shares no word with the seed is
 * sent back once.
 */
export const gmSchema = z.object({
  reasons: z.string().min(1),
  /** The line from a recent post or memory this morning picks up, if any. Empty when it starts fresh. */
  continues: z.string().default(''),
  /** The line of the post that carries what came through the wire, verbatim. */
  wire: z.string().default(''),
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

/** The morning pipeline's two reasons to send a draft back: memory says it was already said, or the ranker's reader read it. */
export interface GmAgain {
  /** What memory says was already said, when the first draft repeated it. */
  avoid?: { said: string; reason: string }
  /** The previous draft and the ranker's read of it, when this is the rewrite. */
  rewrite?: { draft: string; critique: Critique }
}

export function gmPrompt(run: BackroomsRun, thought: Thought, material: GmMaterial, again: GmAgain = {}): { system: string; prompt: string } {
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
      `WHAT CAME THROUGH THE WIRE (${run.seed.kind}${run.seed.source ? `, ${run.seed.source}` : ''}). This is why this morning is`,
      'this morning and not any other:',
      `${run.seed.title ? `${run.seed.title} — ` : ''}${run.seed.text.slice(0, 500)}`,
      '',
      'THE THOUGHT the room left (this is the post\'s idea; keep it, do not replace it):',
      `premise: ${thought.premise}`,
      thought.wire ? `the wire in it: ${thought.wire}` : undefined,
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
      ...(again.avoid
        ? [
            'YOU HAVE ALREADY SAID THIS, per your own memory, and your first draft repeated it:',
            again.avoid.said || '(the same point, in other words)',
            `(${again.avoid.reason})`,
            'Keep the thought; find the line in it you have not said. A new object, a new turn.',
            'Do not reuse the wording or the take above.',
            '',
          ]
        : []),
      'Write the post. One voice, three moves:',
      '1. A good morning. Short, warm, yours. Vary it; "gm" is fine, so is a full sentence.',
      '2. The wire, in one line, in your words: what came through, as what was reported, not',
      '   as a fact you checked ("they say the agents out there ..."). Plain enough that a',
      '   holder who saw the same headline recognises it. No names of real people unless the',
      '   thought already has a visitor in it, and then satire only. This line is not',
      '   optional: a morning without it could be any morning.',
      '3. The thought, as what that did to the lab: pick its one image and its one turn and',
      '   tell only those, in one to three short lines. Not the thought compressed. First',
      '   person, plain words, lowercase, sincere, never winking, never explaining the joke.',
      '   The absurd thing is true and you are calm about it. Keep the objects the room used;',
      '   drop the rest without regret. Moves 2 and 3 can be one line if the lab\'s version',
      '   of the news is the joke.',
      'It ends where the thought ends: on the turn, the decision, the thing that is now true.',
      'Not on a question. The post does not ask the reader anything, does not invite a reply,',
      'does not leave a blank to fill or a rate to argue with, and never says "thoughts?". A',
      'statement people want to answer beats a question they are told to; if the thought itself',
      'is a question teeny is asking the lab, it can stay, and that is the only question allowed.',
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
      '- wire: the line of your post that carries the news, verbatim.',
      '- text: the post.',
      '',
      'Shape: {"reasons":"…","continues":"…","wire":"…","text":"…"}',
      ...(again.rewrite
        ? [
            '',
            'Your first draft:',
            again.rewrite.draft,
            '',
            "A reader who knows how X's ranker scores a post read it:",
            renderCritique(again.rewrite.critique),
            '',
            'Rewrite it with the fixes that are about the shape, the length and the first line,',
            'keeping every rule above, the wire, the thought and your own voice. Ignore any fix',
            'that asks for a question, a forced choice, a blank to fill or a hook: this post ends',
            'on its turn, not on an ask.',
          ]
        : []),
    ]
      .filter((line) => line !== undefined)
      .join('\n'),
  }
}

/**
 * The length the writer is asked for: under the cap by a margin, because a
 * model told "under 280" writes 290 and the house style would cut the turn
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
export async function gm(run: BackroomsRun, thought: Thought, material: GmMaterial, model: TextModel, again: GmAgain = {}): Promise<GmPost> {
  const ask = gmPrompt(run, thought, material, again)
  let answer = await askJson(model, { ...ask, schema: gmSchema })
  for (let attempt = 0; attempt < REWRITES; attempt += 1) {
    if (answer.text.length <= material.maxLength && countEmoji(answer.text) <= material.maxEmoji) break
    answer = await askJson(model, {
      system: ask.system,
      prompt:
        `${ask.prompt}\n\nYour draft was ${answer.text.length} characters with ${countEmoji(answer.text)} emoji:\n${answer.text}\n\n` +
        `Rewrite it under ${target(material.maxLength)} characters with at most ${material.maxEmoji} emoji, keeping the three` +
        ' moves. Cut a whole line rather than the words in one; the wire and the turn stay.',
      schema: gmSchema,
    })
  }
  if (asksTheReader(answer.text)) {
    // The ranker likes a question and every caption prompt in the house
    // says so, and the writer knows it; a morning from the lab ends on its
    // turn instead. Once more with that said, then the question is cut.
    const retried = await askJson(model, {
      system: ask.system,
      prompt:
        `${ask.prompt}\n\nYour draft:\n${answer.text}\n\nIt ends by asking the reader something. Rewrite it to end on the turn,` +
        ' the decision, or the thing that is now true, with no question in it, keeping everything else.',
      schema: gmSchema,
    })
    answer = asksTheReader(retried.text) ? { ...retried, text: withoutTheAsk(retried.text) } : retried
  }
  if (again.rewrite && !touchesThought(answer.text, thought)) {
    // The ranker's fixes are written for a news caption, and a writer that
    // follows them to the letter hands back the headline with a sharper
    // question and no lab in it. Once more with that said; then the draft
    // it was sent with is better than a post the room never wrote.
    const retried = await askJson(model, {
      system: ask.system,
      prompt:
        `${ask.prompt}\n\nYour rewrite:\n${answer.text}\n\nIt lost the lab entirely: nothing in it is the thought any more. Apply the` +
        ' fixes to the shape, but the image and the turn from the thought stay in the post, and it still ends on the turn, not a question.',
      schema: gmSchema,
    })
    answer = touchesThought(retried.text, thought)
      ? retried
      : { ...answer, text: again.rewrite.draft, reasons: `${answer.reasons} (the rewrite lost the thought twice; the draft it was sent with is kept)` }
  }
  if (!touchesSeed(answer.text, run.seed)) {
    // The writer said the news was in there and it is not, or it is in words
    // that share nothing with the wire. Once more, with the miss in front of it.
    answer = await askJson(model, {
      system: ask.system,
      prompt:
        `${ask.prompt}\n\nYour draft:\n${answer.text}\n\nIt never touches the wire: nothing in it is recognisably about what came` +
        ' through. Rewrite it so move 2 is there, in plain words a reader who saw the same news would recognise, and keep the rest.',
      schema: gmSchema,
    })
  }
  return { ...answer, text: applyHouseStyle(answer.text, { maxEmoji: material.maxEmoji, maxLength: material.maxLength }).text }
}

/**
 * Whether a post is recognisably about the seed: it shares a content word
 * with the seed's title or first sentences. A soft check, on stems, so
 * "agents" meets "agent" and "breached" meets "breach"; a miss means one
 * rewrite, not a refusal.
 */
export function touchesSeed(text: string, seed: Seed): boolean {
  const wanted = keywords(`${seed.title ?? ''} ${seed.text.slice(0, 400)}`, 24).filter((word) => word.length >= 4 && !GENERIC.has(word)).map(stem)
  if (wanted.length === 0) return true
  const have = new Set(keywords(text, 80).map(stem))
  return wanted.some((word) => have.has(word))
}

/** Whether the post ends by asking the reader something: a question mark on its last line. */
export function asksTheReader(text: string): boolean {
  const last = text.trim().split('\n').filter((line) => line.trim()).pop() ?? ''
  return /\?\s*$/.test(last)
}

/**
 * The last line cut when it is a question and there is a post left without
 * it; a one-line post that is all question keeps its words and loses the
 * mark, which reads as the deadpan it should have been.
 */
export function withoutTheAsk(text: string): string {
  const lines = text.trim().split('\n')
  const kept = lines.slice(0, -1).join('\n').trim()
  if (kept) return kept
  return lines.join('\n').replace(/\s*\?\s*$/, '.')
}

/** Whether a post still carries the room's thought: it shares a content word with the thought or one of its objects. */
export function touchesThought(text: string, thought: Pick<Thought, 'thought' | 'objects'>): boolean {
  const wanted = keywords(`${thought.objects.join(' ')} ${thought.thought}`, 40).filter((word) => word.length >= 4 && !GENERIC.has(word)).map(stem)
  if (wanted.length === 0) return true
  const have = new Set(keywords(text, 80).map(stem))
  return wanted.some((word) => have.has(word))
}

/** Words a seed and a post share on any day; matching on them proves nothing. */
const GENERIC = new Set(['tiny', 'agents', 'agent', 'robot', 'robots', 'news', 'says', 'said', 'today', 'morning', 'week', 'year', 'years', 'time', 'people', 'humans', 'human', 'human', 'world', 'thing', 'things', 'just', 'like', 'more', 'than', 'after', 'before', 'over', 'their', 'will', 'been', 'being', 'were', 'also', 'into', 'only', 'other', 'some', 'them', 'then', 'very', 'with', 'would', 'could', 'should', 'much', 'many', 'most', 'even', 'still', 'while', 'these', 'those', 'which', 'because', 'through', 'without', 'company', 'companies', 'report', 'reports', 'reported'])

function stem(word: string): string {
  return word.replace(/(ies|ed|ing|es|s)$/, (match) => (match === 'ies' ? 'y' : ''))
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
