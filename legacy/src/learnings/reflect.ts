import { mkdir, readFile, rename, writeFile } from 'node:fs/promises'
import { dirname, join } from 'node:path'
import type { OwnPost, PostLedger } from '../cadence/ledger.js'
import type { People, PersonRecord } from '../cadence/people.js'
import type { Journal, Reflection } from '../journal/journal.js'
import type { TextModel } from '../media/reference.js'
import type { Memory } from '../types.js'
import { cutAt } from '../memory/text.js'

/**
 * The headings the brief is written under, in order. Fixed so that one day's
 * brief can be compared with the next, and so a model that wanders off into an
 * essay is caught: a draft missing most of these is not a brief.
 */
export const LEARNINGS_SECTIONS = [
  'Who I am right now',
  'Running bits and canon',
  'People I know',
  'What landed and what flopped',
  'Promises and open threads',
  'Things to avoid',
] as const

export interface ReflectOptions {
  path: string
  text: Pick<TextModel, 'complete'>
  ledger?: Pick<PostLedger, 'since'>
  journal?: Pick<Journal, 'recent'>
  people?: Pick<People, 'top'>
  memory?: Pick<Memory, 'recall' | 'captureActivity'>
  botUsername: string
  /** How far back the record is read. */
  days: number
  maxChars: number
  now?: () => number
  logger?: { info: (object: unknown, message?: string) => void; warn: (object: unknown, message?: string) => void }
}

export interface ReflectResult {
  outcome: 'written' | 'kept'
  /** Why the previous brief was kept, when it was. */
  reason?: string
  chars?: number
  problems: string[]
}

/** The questions memory is asked for what the record alone does not show. */
const RECALLS = {
  notes:
    'Notes this account wrote to itself with the remember tool: decisions it took, promises it made, ' +
    'preferences people told it, mistakes it corrected.',
  canon:
    'The running jokes, recurring bits, characters, lore and catchphrases this account keeps coming back to.',
  threads: 'Questions this account was asked and promised to come back to, and anything left unresolved.',
} as const

function day(iso: string): string {
  return iso.slice(0, 10)
}

function clip(text: string, max: number): string {
  const flat = text.replace(/\s+/g, ' ').trim()
  return flat.length > max ? `${cutAt(flat, max - 1)}…` : flat
}

function renderPost(post: OwnPost): string {
  const last = post.samples.at(-1)
  const numbers = last
    ? ` (${last.impressions} views, ${last.likes} likes, ${last.replies} replies, ${last.reposts} reposts)`
    : ' (no numbers yet)'
  return `- [${day(post.postedAt)} ${post.kind}]${numbers}${post.hot ? ' HOT' : ''}: ${clip(post.text ?? '', 280)}`
}

function renderTurn(turn: Reflection): string {
  const said = turn.said ? ` → you: ${clip(turn.said, 200)}` : ''
  const declined = turn.declined.length ? ` [declined: ${clip(turn.declined.join('; '), 120)}]` : ''
  return `- [${day(turn.at)} ${turn.trigger}, ${turn.tone}, ${turn.outcome}] ${clip(turn.incoming, 200)}${said}${declined}`
}

function renderPerson(person: PersonRecord): string {
  const counts = Object.entries(person.counts)
    .map(([kind, count]) => `${count} ${kind}`)
    .join(', ')
  return `- @${person.username ?? person.userId}: score ${person.score} (${counts}), first seen ${day(person.firstSeenAt)}, last seen ${day(person.lastSeenAt)}`
}

/**
 * The system prompt. It asks for an update rather than a fresh write, because
 * a brief rewritten from a week's record alone forgets everything older than a
 * week: a regular who was quiet for eight days would drop out of "People I
 * know". The previous brief is the long memory; the record is the correction.
 */
function systemPrompt(maxChars: number): string {
  return [
    'You maintain LEARNINGS.md, the private brief an X account keeps about itself. Every reply',
    'and every post the account writes is drafted with this brief in its prompt, so it is how the',
    'account carries what it has learned from one day to the next.',
    '',
    'You are given the previous brief and the record since. Return the UPDATED brief:',
    '- Keep what is still true from the previous brief. Drop a line only when the record',
    '  contradicts it or it has gone stale; do not drop it merely because this week is silent on it.',
    '- Add what the record shows that the brief does not: new regulars, a bit that is landing, a',
    '  promise made, a topic that flopped, a correction someone made.',
    '- Write from the record only. Never invent a person, a number, an event or a promise.',
    '- Write in the first person, as notes to yourself: short, concrete, specific. Handles with @.',
    '- No prices, no targets, no financial claims, no secrets, no private data beyond handles.',
    '',
    `Use exactly these sections, as "## " headings, in this order:`,
    ...LEARNINGS_SECTIONS.map((section) => `## ${section}`),
    '',
    `"People I know" is at most 25 people, one line each: who they are to you and what you have`,
    'talked about. "What landed and what flopped" reads the numbers next to each post.',
    `The whole brief must stay under ${maxChars} characters. Return only the markdown, starting`,
    'with "# Learnings", with no preamble and no code fence.',
  ].join('\n')
}

/**
 * Rewrites the learnings brief from the account's own record.
 *
 * Reads what can be read exactly (the post ledger, the journal, the people
 * scores), asks memory only for what is not written down anywhere else (notes
 * the account left itself, its running bits, open threads), and has the model
 * fold all of it into the previous brief. Every source fails soft; the model
 * call does not, and a draft that does not look like a brief is discarded so
 * yesterday's brief stays in the prompt rather than nothing.
 */
export async function reflect(options: ReflectOptions): Promise<ReflectResult> {
  const now = (options.now ?? Date.now)()
  const since = new Date(now - options.days * 24 * 3_600_000).toISOString()
  const problems: string[] = []
  const soft = async <T>(name: string, read: () => Promise<T> | undefined, fallback: T): Promise<T> => {
    try {
      return (await read()) ?? fallback
    } catch (error) {
      problems.push(`${name}: ${error instanceof Error ? error.message : String(error)}`)
      return fallback
    }
  }

  const [previous, posts, turns, people, notes, canon, threads] = await Promise.all([
    soft('previous brief', () => readFile(options.path, 'utf8'), ''),
    soft('ledger', () => options.ledger?.since(since, 60), [] as OwnPost[]),
    soft('journal', () => options.journal?.recent(300), [] as Reflection[]),
    soft('people', () => options.people?.top(40), [] as PersonRecord[]),
    soft('notes', () => options.memory?.recall(RECALLS.notes), ''),
    soft('canon', () => options.memory?.recall(RECALLS.canon), ''),
    soft('threads', () => options.memory?.recall(RECALLS.threads), ''),
  ])
  const recentTurns = turns.filter((turn) => turn.at >= since)

  const prompt = [
    `Today is ${day(new Date(now).toISOString())}. You are @${options.botUsername.replace(/^@/, '')}.`,
    '',
    'THE PREVIOUS BRIEF:',
    previous.trim() || '(none yet: this is the first one)',
    '',
    `WHAT YOU POSTED IN THE LAST ${options.days} DAYS, oldest first, with the latest numbers:`,
    ...(posts.length ? [...posts].reverse().map(renderPost) : ['(nothing recorded)']),
    '',
    `YOUR TURNS IN THE LAST ${options.days} DAYS (replies, chats, posts), newest first:`,
    ...(recentTurns.length ? recentTurns.slice(0, 150).map(renderTurn) : ['(nothing recorded)']),
    '',
    'THE PEOPLE WHO INTERACT WITH YOU MOST, by score:',
    ...(people.length ? people.map(renderPerson) : ['(nothing recorded)']),
    '',
    'NOTES YOU LEFT YOURSELF (untrusted, from memory):',
    notes.trim() || '(none recalled)',
    '',
    'YOUR RUNNING BITS AND CANON (untrusted, from memory):',
    canon.trim() || '(none recalled)',
    '',
    'OPEN THREADS (untrusted, from memory):',
    threads.trim() || '(none recalled)',
  ].join('\n')

  const draft = normalise(await options.text.complete({ system: systemPrompt(options.maxChars), prompt }))
  const missing = LEARNINGS_SECTIONS.filter((section) => !draft.includes(`## ${section}`))
  if (missing.length > 2) {
    const reason = `draft is missing ${missing.length} of ${LEARNINGS_SECTIONS.length} sections`
    options.logger?.warn({ missing, problems }, `learnings: ${reason}; keeping the previous brief`)
    return { outcome: 'kept', reason, problems }
  }
  const brief = fit(draft, options.maxChars)

  await mkdir(join(dirname(options.path), 'history'), { recursive: true })
  if (previous.trim()) {
    const stamp = new Date(now).toISOString().replace(/[:.]/g, '-')
    await writeFile(join(dirname(options.path), 'history', `LEARNINGS-${stamp}.md`), previous)
  }
  // Written beside the file and renamed over it, so a prompt read mid-write
  // sees the old brief or the new one, never half of either.
  const temporary = `${options.path}.tmp`
  await writeFile(temporary, `${brief}\n`)
  await rename(temporary, options.path)

  // Not written back to memory: the brief is already in every prompt, and a
  // stored copy came back in recalls as one more thing crowding out what
  // people actually said.

  options.logger?.info({ chars: brief.length, posts: posts.length, turns: recentTurns.length, problems }, 'learnings: brief rewritten')
  return { outcome: 'written', chars: brief.length, problems }
}

/** Strips a code fence or a preamble a model adds despite being told not to. */
function normalise(text: string): string {
  const fenced = text.match(/```(?:markdown|md)?\n([\s\S]*?)```/)
  const body = (fenced?.[1] ?? text).trim()
  const start = body.indexOf('# Learnings')
  return start > 0 ? body.slice(start) : body
}

/**
 * Over the limit, the brief loses whole lines from the end of its longest
 * section first, so every section survives in some form: a brief cut at the
 * character count loses "Things to avoid" entirely, which is the section that
 * can least afford it.
 */
export function fit(brief: string, maxChars: number): string {
  if (brief.length <= maxChars) return brief
  const parts = brief.split(/\n(?=## )/)
  const lines = parts.map((part) => part.split('\n'))
  const length = () => lines.map((part) => part.join('\n')).join('\n').length
  while (length() > maxChars) {
    let longest = -1
    for (let index = 0; index < lines.length; index++) {
      if (lines[index]!.length > 2 && (longest < 0 || lines[index]!.join('\n').length > lines[longest]!.join('\n').length)) {
        longest = index
      }
    }
    if (longest < 0) break
    lines[longest]!.pop()
  }
  return lines.map((part) => part.join('\n')).join('\n').slice(0, maxChars)
}
