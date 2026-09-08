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

export function loadPersona(
  options: { soulPath?: string; contextPath?: string; skillsDir?: string } = {},
): Persona {
  const soul = options.soulPath ? readIfPresent(options.soulPath) : undefined
  const context = options.contextPath ? readIfPresent(options.contextPath) : undefined
  return {
    ...(soul ? { soul } : {}),
    ...(context ? { context } : {}),
    skills: options.skillsDir ? readSkills(options.skillsDir) : [],
  }
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
