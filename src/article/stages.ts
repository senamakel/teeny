import { z } from 'zod'
import { PRICE } from '../backrooms/evaluate.js'
import type { Thought } from '../backrooms/distill.js'
import type { BackroomsRun } from '../backrooms/types.js'
import { VOICE, material } from '../backrooms/variants.js'

/**
 * The text steps of an article, as prompts and the shapes they answer in.
 *
 * The order is the point: a structure is written and checked before a word
 * of prose exists, then each section is written against its own purpose
 * and beats with the previous sections in view, and only then is the whole
 * thing read by the proof-reader (proofread.ts). Nothing here calls a
 * model; the pipeline does, so every prompt is testable as text.
 */

export const ARTICLE_VOICE = [
  VOICE,
  'you are writing a long piece for teeny\'s blog, published as an article on x. lore only:',
  'nothing about price, market cap, or what anyone should buy. the reader is a stranger who',
  'has never seen the lab; they should be able to follow it on the first read.',
].join(' ')

/** What every prompt is written from. */
export interface ArticleMaterial {
  run: BackroomsRun
  thought: Thought
  /** The engine tonight is about, one of the lore's; absent when the room drew its own. */
  engine?: string
  /** An editorial note the piece is held to, e.g. that it is the first one and introduces everything. */
  brief?: string
  /** The lore as teeny may know it: goals, cast, bits, and only the latent items already surfaced. */
  lore: string
}

export const imageBriefSchema = z.object({
  /** The single frame an image model draws: one location, teeny, at most three moving things, no text in frame. */
  brief: z.string().min(1),
  /** Under the picture, teeny's words, lowercase. */
  caption: z.string().max(200).default(''),
})

export const structureSchema = z.object({
  reasons: z.string().min(1),
  /** Lowercase, under 60 characters, no colon-subtitle. */
  title: z.string().min(1).max(80),
  /** One sentence under the title. */
  dek: z.string().min(1).max(240),
  /** One line: what the piece is about, and what the turn is. */
  angle: z.string().min(1),
  cover: imageBriefSchema,
  sections: z
    .array(
      z.object({
        key: z.string().default(''),
        heading: z.string().min(1).max(80),
        /** One line: what this section does for the story. */
        purpose: z.string().min(1),
        /** Two to five things that happen, in order, each something that can be shown. */
        beats: z.array(z.string().min(1)).min(2).max(5),
        /** Target length of this section. */
        words: z.number().int().min(80).max(800),
        image: imageBriefSchema,
      }),
    )
    .min(3)
    .max(5),
  /** One line: the last image the reader is left with. */
  closing: z.string().min(1),
})

export type Structure = z.infer<typeof structureSchema>

export const structureCheckSchema = z.object({
  ok: z.boolean(),
  problems: z.array(z.string()).default([]),
  /** What to change, one line each, when not ok. */
  fixes: z.array(z.string()).default([]),
})

export type StructureCheck = z.infer<typeof structureCheckSchema>


/** Section keys are positional whatever the model wrote, and word targets sum to the target. */
export function normaliseStructure(structure: Structure, words: number): Structure {
  const sum = structure.sections.reduce((total, section) => total + section.words, 0) || 1
  const scale = words / sum
  return {
    ...structure,
    // The voice is lowercase; a plan that came back in title case is not.
    title: structure.title.trim().toLowerCase(),
    dek: structure.dek.trim().toLowerCase(),
    cover: { ...structure.cover, caption: structure.cover.caption.trim().toLowerCase() },
    sections: structure.sections.map((section, index) => ({
      ...section,
      key: `s${index + 1}`,
      words: Math.max(80, Math.round(section.words * scale)),
      image: { ...section.image, caption: section.image.caption.trim().toLowerCase() },
    })),
  }
}

export function structurePrompt(
  m: ArticleMaterial,
  options: { sectionsMax: number; words: number },
): { system: string; prompt: string } {
  return {
    system: `${ARTICLE_VOICE} right now you are planning the piece, not writing it.`,
    prompt: [
      ...material(m.run, m.thought, 8),
      '',
      ...(m.engine
        ? [`tonight the story is about ${m.engine}`, '(that is a note to you, the writer, about which part of the lore to lean on; the words "engine", "tonight" and "the others rest" are not in the story.)', '']
        : []),
      ...(m.brief ? ['the brief for this piece, from the editor:', m.brief, ''] : []),
      'what you may draw on (and nothing that has not come out yet):',
      m.lore,
      '',
      `plan an article of about ${options.words} words as one story that happened in the lab, told in`,
      `order, that ends on the turn. three to ${options.sectionsMax} sections, each a scene with one`,
      'purpose, two to five beats that can be shown rather than explained, a word target (the targets',
      `add up to ${options.words}), and one image brief: the single frame an image model draws for`,
      'that scene, specific about the room, the light, where teeny is, what the tiny agents carry;',
      'say "teeny" and "the tiny agents" and nothing about what they look like (the image model',
      'has a sheet for that; a brief that describes a mug or a box gets a mug or a box drawn);',
      'no text, letters, numbers or writing anywhere in the frame. the cover is the whole piece in',
      'one frame, the same rules. the title is lowercase, under 60 characters, no colon. the dek is',
      'one sentence a stranger reads before deciding to click. the angle says what the piece is',
      'about and what the turn is. the closing is the last image the reader is left with.',
      '',
      'reasons first: which idea from the room you built the piece on and why.',
      '',
      'Shape: {"reasons":"…","title":"…","dek":"…","angle":"…","cover":{"brief":"…","caption":"…"},',
      '"sections":[{"key":"s1","heading":"…","purpose":"…","beats":["…","…"],"words":300,',
      '"image":{"brief":"…","caption":"…"}}],"closing":"…"}',
    ].join('\n'),
  }
}

export function checkStructurePrompt(structure: Structure, m: ArticleMaterial): { system: string; prompt: string } {
  return {
    system: [
      'you are a cold, exact editor reading the plan for a piece of serial fiction before anyone',
      'writes it. you do not write; you say whether the plan will hold and what to change.',
    ].join(' '),
    prompt: [
      'the plan:',
      renderStructure(structure),
      '',
      ...(m.brief ? ['the brief the plan must serve:', m.brief, ''] : []),
      'what came through the wire, which the piece may use freely (real places, companies and',
      'institutions in the news are fine; real people only as obvious satire):',
      `${m.run.seed.title ? `${m.run.seed.title} — ` : ''}${m.run.seed.text.slice(0, 400)}`,
      '',
      'the lore it must stay inside:',
      m.lore,
      '',
      'answer these, and only these: does it have a beginning, a turn and an ending, in that order?',
      'do any two sections do the same job? is every beat something that can be shown in a scene',
      'rather than explained? does it invent lore that is not in the list above? does it give a',
      'price, a market cap, or advice about buying anything? is the title lowercase and under 60',
      'characters? ok is true only when every answer is the right one. problems are what is wrong,',
      'one line each; fixes are what to change, one line each.',
      '',
      'Shape: {"ok":true,"problems":[],"fixes":[]}',
    ].join('\n'),
  }
}

export function reviseStructurePrompt(
  structure: Structure,
  check: StructureCheck,
  m: ArticleMaterial,
  options: { sectionsMax: number; words: number },
): { system: string; prompt: string } {
  const base = structurePrompt(m, options)
  return {
    system: base.system,
    prompt: [
      'your plan was:',
      renderStructure(structure),
      '',
      'the editor said:',
      ...check.problems.map((line) => `- ${line}`),
      ...check.fixes.map((line) => `- fix: ${line}`),
      '',
      'plan it again with those fixed and everything else kept. the same shape as before:',
      '',
      base.prompt.split('\nShape:')[1] ? `Shape:${base.prompt.split('\nShape:')[1]}` : '',
    ].join('\n'),
  }
}

export function sectionPrompt(
  structure: Structure,
  index: number,
  previous: string[],
  m: ArticleMaterial,
): { system: string; prompt: string } {
  const section = structure.sections[index]!
  const count = structure.sections.length
  return {
    system: `${ARTICLE_VOICE} you are writing one section of a piece you have already planned.`,
    prompt: [
      `the piece: "${structure.title}" — ${structure.dek}`,
      `the angle: ${structure.angle}`,
      ...(m.brief ? [`the brief: ${m.brief}`] : []),
      `the closing image, which the last section lands on: ${structure.closing}`,
      '',
      'the thought it came from, in your words:',
      m.thought.thought,
      '',
      ...(previous.length > 0
        ? ['what is already written, in order (do not repeat any of it):', ...previous.flatMap((body, at) => [`## ${structure.sections[at]!.heading}`, body, '']), '']
        : []),
      `write section ${index + 1} of ${count}: "${section.heading}".`,
      `its purpose: ${section.purpose}`,
      'its beats, in order:',
      ...section.beats.map((beat) => `- ${beat}`),
      '',
      `about ${section.words} words. markdown, whole paragraphs, cause and effect, in teeny's voice.`,
      'no heading line (it is added), no image (it is added), no list unless the beats are a list,',
      'no sub-headings, no summary of what came before, no explaining the joke. the beats are what',
      'happens; write them as scenes with the objects in them.',
      '',
      'answer with the section\'s text and nothing else: no title, no preamble, no code fence.',
    ].join('\n'),
  }
}

export function tightenPrompt(structure: Structure, index: number, body: string, target: number): { system: string; prompt: string } {
  const section = structure.sections[index]!
  return {
    system: `${ARTICLE_VOICE} you are cutting a section you wrote down to length.`,
    prompt: [
      `section "${section.heading}" is ${wordCount(body)} words; it should be about ${target}.`,
      'cut it to that length. keep every beat, keep the order, keep the voice; lose repetition,',
      'throat-clearing and the sentences that explain what a scene already showed. markdown, no',
      'heading line, no image.',
      '',
      body,
      '',
      'answer with the cut section\'s text and nothing else: no title, no preamble, no code fence.',
    ].join('\n'),
  }
}

/** The plan as a reader sees it, for the check and the dry run. */
export function renderStructure(structure: Structure): string {
  return [
    `title: ${structure.title}`,
    `dek: ${structure.dek}`,
    `angle: ${structure.angle}`,
    `cover: ${structure.cover.brief}`,
    '',
    ...structure.sections.flatMap((section, index) => [
      `${index + 1}. ${section.heading} (${section.words} words)`,
      `   purpose: ${section.purpose}`,
      ...section.beats.map((beat) => `   - ${beat}`),
      `   image: ${section.image.brief}`,
      '',
    ]),
    `closing: ${structure.closing}`,
  ].join('\n')
}

/** The dek, then each section under its heading with the picture right under that. */
export function assembleMarkdown(structure: Structure, bodies: string[]): string {
  return [
    `_${structure.dek.trim()}_`,
    '',
    ...structure.sections.flatMap((section, index) => [
      `## ${section.heading.trim()}`,
      '',
      `![${section.image.caption.replace(/[[\]]/g, '')}](image:${section.key})`,
      '',
      stripSectionExtras(bodies[index] ?? '').trim(),
      '',
    ]),
  ]
    .join('\n')
    .trim()
}

/** A model asked for a body alone sometimes adds the heading or a picture anyway. */
export function stripSectionExtras(body: string): string {
  return body
    .split('\n')
    .filter((line) => !/^#{1,6}\s/.test(line.trim()) && !/^!\[[^\]]*\]\([^)]*\)\s*$/.test(line.trim()))
    .join('\n')
    .trim()
}

/** The dek as the article carries it: the first line, when it is the italic one `assembleMarkdown` put there. */
export function dekOf(markdown: string): string | undefined {
  const first = markdown.split('\n').find((line) => line.trim())?.trim()
  const match = first ? /^_(.+)_$/.exec(first) : undefined
  return match?.[1]?.trim()
}

/** The markdown without that line, for a page whose header already shows the dek. */
export function withoutDek(markdown: string): string {
  const lines = markdown.split('\n')
  const at = lines.findIndex((line) => line.trim())
  if (at < 0 || !/^_(.+)_$/.test(lines[at]!.trim())) return markdown
  return lines
    .slice(at + 1)
    .join('\n')
    .trim()
}

export function wordCount(markdown: string): number {
  return markdown
    .replace(/!\[[^\]]*\]\([^)]*\)/g, '')
    .split(/\s+/)
    .filter((word) => /[a-z0-9]/i.test(word)).length
}

const URL = /https?:\/\/\S+|\bwww\.\S+/gi
const HANDLE = /(^|[^a-z0-9_])@[a-z0-9_]{2,}/gi
const HASHTAG = /(^|\s)#[a-z0-9_]+/gi

/**
 * The voice rules the model is told and the article is held to anyway: no
 * urls, no handles, no hashtags. `$TINY` is teeny's own coin and stays; a
 * price, a market cap or "nfa" is not cut, it is reported, because an
 * article that talks money is not this article.
 */
export function articleStyle(markdown: string): { text: string; fixes: string[]; priced: string[] } {
  const fixes: string[] = []
  let text = markdown
  const urls = text.match(URL) ?? []
  if (urls.length > 0) {
    text = text.replace(URL, '')
    fixes.push(`cut ${urls.length} url(s)`)
  }
  const handles = text.match(HANDLE) ?? []
  if (handles.length > 0) {
    text = text.replace(HANDLE, '$1')
    fixes.push(`cut ${handles.length} handle(s)`)
  }
  const hashtags = text.match(HASHTAG) ?? []
  if (hashtags.length > 0) {
    text = text.replace(HASHTAG, '$1')
    fixes.push(`cut ${hashtags.length} hashtag(s)`)
  }
  text = text.replace(/[ \t]+$/gm, '').replace(/ {2,}/g, ' ')
  const priced = text
    .split('\n')
    .filter((line) => PRICE.test(line))
    .map((line) => line.trim().slice(0, 120))
  return { text, fixes, priced }
}
