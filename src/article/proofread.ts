import { readFileSync } from 'node:fs'
import { z } from 'zod'
import type { TextModel } from '../media/reference.js'
import { askJson } from '../morning/stages.js'
import { IMAGE_PLACEHOLDER } from './content-state.js'

/**
 * The proof-reader: the humanizer skill, read twice.
 *
 * First as a reviewer. The skill is a catalogue of the ways machine prose
 * gives itself away; the model reads the article against it and lists
 * every place it trips one, with the line and the fix, and nothing else.
 * Then, only when there is something to fix, as an editor applying exactly
 * those findings. Reading and fixing are two calls so the findings are on
 * the record: a dry run shows what the proof-reader saw, and whether the
 * fix helped, before anything is published.
 */

export const findingSchema = z.object({
  /** Where: the section key (s1, s2, …) or "dek". */
  section: z.string().default(''),
  /** Which of the skill's numbered patterns, e.g. "14 em dashes". */
  pattern: z.string().min(1),
  /** The words as written, short. */
  quote: z.string().min(1),
  /** What to write instead, or how. */
  fix: z.string().min(1),
})

export const proofSchema = z.object({
  verdict: z.enum(['clean', 'fix']),
  findings: z.array(findingSchema).default([]),
  /** One line: does it read like the writer whose sample was given. */
  voice: z.string().default(''),
})

export type Proof = z.infer<typeof proofSchema>
export type Finding = z.infer<typeof findingSchema>

/** The skill file without its frontmatter, which is for a skill loader and not the model. */
export function loadHumanizer(path: string): string {
  return stripFrontmatter(readFileSync(path, 'utf8'))
}

export function stripFrontmatter(text: string): string {
  const match = /^---\n[\s\S]*?\n---\n/.exec(text)
  return (match ? text.slice(match[0].length) : text).trim()
}

const PROOF_NOTE = [
  'you are proof-reading, not rewriting. the text below is markdown fiction in the voice of a',
  'small robot: lowercase, plain words, short sentences, sincere. that voice is deliberate and',
  'is not a finding. the skill above is your checklist. read the whole text against it and list',
  'every place it trips one of the numbered patterns: which pattern, the words as written, and',
  'the fix. the "what not to flag" section applies. verdict is "clean" when there is nothing',
  'worth changing; "fix" otherwise. one line on whether it reads like the writing sample.',
].join(' ')

export function proofPrompt(skill: string, markdown: string, sample: string): { system: string; prompt: string } {
  return {
    system: `${skill}\n\n---\n\n${PROOF_NOTE}`,
    prompt: [
      'a writing sample, the same writer, for the voice:',
      sample,
      '',
      'the text:',
      '',
      markdown,
      '',
      'Shape: {"verdict":"clean"|"fix","findings":[{"section":"s1","pattern":"…","quote":"…","fix":"…"}],"voice":"…"}',
    ].join('\n'),
  }
}

const FIX_NOTE = [
  'apply exactly the findings listed, and nothing else. keep every claim, every scene and every',
  'detail as written. keep every `## ` heading line and every `![…](image:…)` line exactly where',
  'they are, unchanged. keep the lowercase voice. pasted-text mode: return only the rewritten',
  'markdown, no preamble, no note on what changed.',
].join(' ')

export function proofFixPrompt(
  skill: string,
  markdown: string,
  findings: Finding[],
  sample: string,
): { system: string; prompt: string } {
  return {
    system: `${skill}\n\n---\n\n${FIX_NOTE}`,
    prompt: [
      'a writing sample, the same writer, for the voice:',
      sample,
      '',
      'the findings:',
      ...findings.map((finding) => `- [${finding.section || '?'}] ${finding.pattern}: "${finding.quote}" → ${finding.fix}`),
      '',
      'the text:',
      '',
      markdown,
    ].join('\n'),
  }
}

/**
 * Whether the fixed text is still the same article: same headings in the
 * same order, every picture still placed, not gutted, and not the skill's
 * one-line "nothing to fix" answer. The reason it is not, when it is not.
 */
export function structurallySame(before: string, after: string): string | undefined {
  const trimmed = after.trim()
  if (!trimmed) return 'the fix came back empty'
  if (/^(nothing to fix|no changes|the text is fine|no fixes)/i.test(trimmed)) return 'the fix said there was nothing to fix'
  if (trimmed.length < before.trim().length * 0.6) return `the fix is ${Math.round((trimmed.length / before.trim().length) * 100)}% of the length`
  const headings = (text: string): string[] =>
    text
      .split('\n')
      .filter((line) => /^##\s/.test(line))
      .map((line) => line.trim())
  const beforeHeadings = headings(before)
  const afterHeadings = headings(after)
  if (beforeHeadings.join('\n') !== afterHeadings.join('\n')) return 'the headings changed'
  const placeholders = (text: string): string[] =>
    text
      .split('\n')
      .map((line) => line.trim())
      .filter((line) => IMAGE_PLACEHOLDER.test(line))
      .map((line) => IMAGE_PLACEHOLDER.exec(line)![2]!)
  if (placeholders(before).join(',') !== placeholders(after).join(',')) return 'an image placeholder moved or went missing'
  return undefined
}

export interface ProofResult {
  markdown: string
  /** What the reviewer saw before any fix. */
  review: Proof
  /** Whether the fix was applied; false when there was nothing to fix or the fix was refused. */
  applied: boolean
  /** Why the fix was refused, when it was. */
  refused?: string
  /** What the reviewer saw after the fix, when one was applied. */
  after?: Proof
}

export async function proofread(
  models: { reviewer: TextModel; fixer: TextModel },
  input: { skill: string; markdown: string; sample: string },
): Promise<ProofResult> {
  const review = await askJson(models.reviewer, { ...proofPrompt(input.skill, input.markdown, input.sample), schema: proofSchema })
  if (review.verdict === 'clean' || review.findings.length === 0) {
    return { markdown: input.markdown, review, applied: false }
  }
  const raw = await models.fixer.complete(proofFixPrompt(input.skill, input.markdown, review.findings, input.sample))
  const fixed = unfence(raw)
  const refused = structurallySame(input.markdown, fixed)
  if (refused) return { markdown: input.markdown, review, applied: false, refused }
  let after: Proof | undefined
  try {
    after = await askJson(models.reviewer, { ...proofPrompt(input.skill, fixed, input.sample), schema: proofSchema })
  } catch {
    // The second read is for the record; the fix stands without it.
  }
  return { markdown: fixed, review, applied: true, ...(after ? { after } : {}) }
}

/** A model returning markdown sometimes wraps it in a fence anyway. */
export function unfence(text: string): string {
  const match = /^\s*```(?:markdown|md)?\s*\n([\s\S]*?)\n```\s*$/.exec(text)
  return (match ? match[1]! : text).trim()
}
