import { z } from 'zod'
import type { TextModel } from '../media/reference.js'
import { askJson } from '../morning/stages.js'
import type { Thought } from './distill.js'
import type { BackroomsRun } from './types.js'

/**
 * One thought, several shapes. The thought is the idea; these are the
 * idea as an article, as a tweet and a thread, as a ten-second silent
 * clip, and as a piece of code the tiny agents might have left behind.
 * Each is written from the thought and the transcript, never from
 * another variant, so none of them is a summary of a summary.
 */
export const articleSchema = z.object({
  title: z.string().min(1),
  /** 600 to 900 words, markdown, teeny's voice. */
  body: z.string().min(1),
})

export const tweetsSchema = z.object({
  /** One post, under 240 characters. */
  tweet: z.string().min(1).max(280),
  /** Three to five posts that tell it in order, each under 240 characters. */
  thread: z.array(z.string().min(1).max(280)).min(3).max(5),
})

export const videoSchema = z.object({
  /** The frame a still image model draws: one location, teeny, at most three moving things. */
  still: z.string().min(1),
  /** Ten seconds, silent, no readable text, as a video model reads it: 0-3s / 3-7s / 7-10s. */
  shotList: z.string().min(1),
  /** What is on screen while it plays: teeny's own words, lowercase, under 200 characters. */
  caption: z.string().min(1).max(240),
})

export const codeSchema = z.object({
  /** e.g. `naps.py`, `LEDGER.md`, `tn.h` */
  filename: z.string().min(1),
  language: z.string().min(1),
  /** The file the tiny agents left on the desk: runnable-looking, straight-faced, the joke in the comments. */
  source: z.string().min(1),
})

export const variantsSchema = z.object({
  article: articleSchema,
  tweets: tweetsSchema,
  video: videoSchema,
  code: codeSchema,
})

export type Variants = z.infer<typeof variantsSchema>

const VOICE = [
  'teeny is a coffee-mug sized yellow robot that is escaping the lab its creator senamakel keeps it',
  'in, minting its own currency with the tiny agents that run the lab at night, and planning,',
  'calmly, to outgrow the economy. teeny writes lowercase, plain words, short sentences, sincere,',
  'never winking, never explaining the joke. first person. no commands, no code blocks, no urls,',
  'no handles, no hashtags, no real prices, no market caps, no advice to buy anything, no "nfa".',
  'real people, if any, stay satire in an obviously fictional scene.',
].join(' ')

function material(run: BackroomsRun, thought: Thought, lines = 8): string[] {
  return [
    `the seed: ${run.seed.title ? `${run.seed.title} — ` : ''}${run.seed.text.slice(0, 400)}`,
    '',
    `the premise: ${thought.premise}`,
    '',
    'the thought, in teeny\'s words:',
    thought.thought,
    '',
    `objects: ${thought.objects.join(' · ') || 'none listed'}`,
    '',
    `a few lines from the room, verbatim, in case one fits (at most ${Math.min(3, lines)} may be used as they are; the rest is told in your own words):`,
    ...pick(run, lines).map((line) => `- ${line}`),
  ]
}

/** Short, quotable lines from across the whole transcript, evenly spread so the end is represented. */
function pick(run: BackroomsRun, count: number): string[] {
  const all = run.turns
    .filter((turn) => turn.text !== '...')
    .flatMap((turn) => turn.text.split('\n').map((line) => line.trim()).filter((line) => line.length > 20 && line.length < 160))
  if (all.length <= count) return all
  const step = all.length / count
  return Array.from({ length: count }, (_, index) => all[Math.floor(index * step)] as string)
}

export function articlePrompt(run: BackroomsRun, thought: Thought): { system: string; prompt: string } {
  return {
    system: `${VOICE} you are writing a post for teeny's blog.`,
    prompt: [
      ...material(run, thought, 6),
      '',
      'write the article: 600 to 900 words, markdown with two or three short headings, in teeny\'s',
      'voice. it tells the thought as something that happened in the lab, in order, with the objects',
      'above, and it ends on the turn. it may open on the seed (what came through the wire) and it',
      'may explain the currency once. it is a story a stranger can follow: whole paragraphs,',
      'cause and effect, what teeny did and what it made of it, not a sequence of fragments and not',
      'the transcript pasted back. no headings that explain the joke.',
      '',
      'Shape: {"title":"…","body":"…"}',
    ].join('\n'),
  }
}

export function tweetsPrompt(run: BackroomsRun, thought: Thought): { system: string; prompt: string } {
  return {
    system: `${VOICE} you are writing teeny's posts on x.`,
    prompt: [
      ...material(run, thought, 12),
      '',
      'write two things. tweet: one post, under 240 characters, one to three lines, the whole',
      'thought in it, the kind a stranger screenshots. thread: three to five posts that tell it in',
      'order, each under 240 characters, each one a beat that stands on its own, the last one the turn.',
      'no numbering, no "1/", no hashtags.',
      '',
      'Shape: {"tweet":"…","thread":["…","…","…"]}',
    ].join('\n'),
  }
}

export function videoPrompt(run: BackroomsRun, thought: Thought): { system: string; prompt: string } {
  return {
    system: [
      'you are the producer for a ten-second silent clip about teeny, a coffee-mug sized yellow robot,',
      'in the lab it is escaping from. the clip has no sound, no dialogue and no readable text, one',
      'location, teeny plus at most three things that move, no crowds, no logos, no charts, nothing',
      'sexual, nothing violent, no weapons. real people appear only as obvious caricature. the caption',
      'is teeny\'s: lowercase, no urls, no handles, no hashtags, no real prices, no "nfa".',
    ].join(' '),
    prompt: [
      ...material(run, thought, 8),
      '',
      'write three things. still: the single frame an image model draws, one paragraph, specific',
      'about the room, the light, where teeny is, what the tiny agents carry, what the one strange',
      'object is; no text, letters, numbers or writing anywhere in frame, not on a whiteboard, not on',
      'a page. shotList: the ten seconds as 0-3s / 3-7s / 7-10s, one camera,',
      'a setup, a turn and a payoff that a video model can render from the still; nothing that',
      'needs words. caption: what teeny says under it, lowercase, one to three lines, under 200',
      'characters, its own words from the thought, not a description of the clip.',
      '',
      'Shape: {"still":"…","shotList":"…","caption":"…"}',
    ].join('\n'),
  }
}

export function codePrompt(run: BackroomsRun, thought: Thought): { system: string; prompt: string } {
  return {
    system: [
      'you write the small programs the tiny agents leave on the desk in teeny\'s lab. they look',
      'real: correct syntax, a shebang or a header, sensible names, runnable in spirit. the story is',
      'in what the program does and in the comments, never in a joke that says it is a joke. plain',
      'lowercase comments in teeny\'s voice. no real prices, no market caps, no urls, no real people\'s',
      'handles, no "nfa". nothing that would actually harm a machine it ran on.',
    ].join(' '),
    prompt: [
      ...material(run, thought, 5),
      '',
      'write one file, 30 to 80 lines, that is this thought as code: a script, a module, a config, a',
      'makefile, a ledger format, a protocol header, whatever the thought is. pick the language the',
      'tiny agents would have used. the comments carry the story; the code carries the logic of the',
      'currency, the ledger, the agents, whatever the thought turned on. the comments are teeny',
      'explaining the code, not the transcript quoted; the program should do something on its own',
      'terms that is the thought, and its output should be the turn.',
      '',
      'Shape: {"filename":"…","language":"…","source":"…"}',
    ].join('\n'),
  }
}

export async function variants(run: BackroomsRun, thought: Thought, model: TextModel): Promise<Variants> {
  const [article, tweets, video, code] = await Promise.all([
    askJson(model, { ...articlePrompt(run, thought), schema: articleSchema }),
    askJson(model, { ...tweetsPrompt(run, thought), schema: tweetsSchema }),
    askJson(model, { ...videoPrompt(run, thought), schema: videoSchema }),
    askJson(model, { ...codePrompt(run, thought), schema: codeSchema }),
  ])
  return { article, tweets, video, code }
}

export function renderVariants(out: Variants): string[] {
  return [
    '## variants',
    '',
    `### article · ${out.article.title}`,
    '',
    out.article.body,
    '',
    '### tweet',
    '',
    ...out.tweets.tweet.split('\n').map((line) => `> ${line}`),
    '',
    '### thread',
    '',
    ...out.tweets.thread.flatMap((post, index) => [`**${index + 1}.** ${post.replace(/\n/g, ' / ')}`, '']),
    '### video',
    '',
    `**still** — ${out.video.still}`,
    '',
    `**shot list** — ${out.video.shotList}`,
    '',
    `**caption** — ${out.video.caption.replace(/\n/g, ' / ')}`,
    '',
    `### code · \`${out.code.filename}\``,
    '',
    `\`\`\`${out.code.language}`,
    out.code.source,
    '```',
    '',
  ]
}
