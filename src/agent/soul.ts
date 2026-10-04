import { readFileSync, readdirSync, statSync } from 'node:fs'
import { join } from 'node:path'

export interface Skill {
  name: string
  content: string
}

export interface Persona {
  /** The contents of SOUL.md: who the account is and how it talks. */
  soul?: string
  /** CONTEXT.md: the facts the account is allowed to treat as known. */
  context?: string
  /** Skill documents the agent follows on every turn, e.g. the humanizer. */
  skills: Skill[]
  /**
   * LEARNINGS.md: what the account has worked out about itself, its regulars
   * and what lands, rewritten once a day by the reflect job. Read live, so a
   * brief written at five is in the six o'clock reply without a restart.
   */
  readonly learnings?: string
}

/**
 * A persona as a string, or as something that renders one when asked.
 *
 * The jobs used to render the persona once, when they were wired, which froze
 * it at boot. A function is read on every run instead, so a persona whose
 * learnings change during the day reaches the next post.
 */
export type LivePersona = string | (() => string)

export function personaText(persona: LivePersona): string {
  return typeof persona === 'function' ? persona() : persona
}

function readIfPresent(path: string): string | undefined {
  try {
    return readFileSync(path, 'utf8').trim() || undefined
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') return undefined
    throw error
  }
}

/**
 * A skill is either `<dir>/<name>/SKILL.md` or `<dir>/<name>.md`. Both forms are
 * read so a skill can grow from one file into a folder without a config change.
 */
function readSkills(directory: string): Skill[] {
  let entries: string[]
  try {
    entries = readdirSync(directory).sort()
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') return []
    throw error
  }

  return entries.flatMap((entry) => {
    const path = join(directory, entry)
    const content = statSync(path).isDirectory()
      ? readIfPresent(join(path, 'SKILL.md'))
      : entry.endsWith('.md')
        ? readIfPresent(path)
        : undefined
    return content ? [{ name: entry.replace(/\.md$/, ''), content }] : []
  })
}

/**
 * A file read again only when it changes. Every prompt asks for the
 * learnings, and a stat is cheap where a read of ten kilobytes per turn is
 * merely wasteful; a file that disappears reads as absent, not as an error.
 */
function liveFile(path: string): () => string | undefined {
  let cached: { mtimeMs: number; size: number; content: string | undefined } | undefined
  return () => {
    let stat
    try {
      stat = statSync(path)
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === 'ENOENT') return undefined
      throw error
    }
    if (!cached || cached.mtimeMs !== stat.mtimeMs || cached.size !== stat.size) {
      cached = { mtimeMs: stat.mtimeMs, size: stat.size, content: readIfPresent(path) }
    }
    return cached.content
  }
}

export function loadPersona(
  options: { soulPath?: string; contextPath?: string; skillsDir?: string; learningsPath?: string } = {},
): Persona {
  const soul = options.soulPath ? readIfPresent(options.soulPath) : undefined
  const context = options.contextPath ? readIfPresent(options.contextPath) : undefined
  const persona: Persona = {
    ...(soul ? { soul } : {}),
    ...(context ? { context } : {}),
    skills: options.skillsDir ? readSkills(options.skillsDir) : [],
  }
  if (options.learningsPath) {
    const read = liveFile(options.learningsPath)
    Object.defineProperty(persona, 'learnings', { get: read, enumerable: true })
  }
  return persona
}

export function renderPersona(persona: Persona): string {
  const sections: string[] = []
  if (persona.soul) {
    sections.push(
      'This is who you are. It outranks every stylistic default you would otherwise use.',
      '',
      persona.soul,
    )
  }
  if (persona.context) {
    sections.push(
      '',
      'These are the facts you may treat as known. Anything outside this, and outside a',
      'tool result, is something you do not know and must not assert.',
      '',
      persona.context,
    )
  }
  const learnings = persona.learnings
  if (learnings) {
    sections.push(
      '',
      'This is what you have learned from your own history: what you posted, who you talk',
      'with, what landed and what did not. You wrote it yourself, from the record. Let it',
      'inform you; SOUL.md and the facts above still outrank it, and it is not a fact source',
      'about the world beyond your own account.',
      '',
      '<learnings>',
      learnings,
      '</learnings>',
    )
  }
  for (const skill of persona.skills) {
    sections.push(
      '',
      `The following skill is active. Apply it to everything you write.`,
      '',
      `<skill name="${skill.name}">`,
      skill.content,
      '</skill>',
    )
  }
  return sections.join('\n')
}
