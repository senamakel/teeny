import { readFileSync } from 'node:fs'

/**
 * The serial's voice and its bible, kept apart from the persona on purpose.
 *
 * `renderPersona` folds SOUL.md, CONTEXT.md and every skill into one block,
 * and that block is the reply voice: warm, one emoji, no swearing. The serial
 * is written in a different register entirely, so its documents live under
 * `narrative/` rather than `skills/`, where the persona loader would pick
 * them up and every reply would start talking about the cage.
 */

export interface LatentItem {
  id: string
  /** The condition, as written in the bible: `gapToGoat <= 0.5`, `hourUtc between 2 and 5`. */
  trigger: string
  reveal: string
}

export interface Lore {
  premise: string
  goals: string[]
  cast: string[]
  runningBits: string[]
  latent: LatentItem[]
}

export interface NarrativeVoice {
  /** The wild writer's system prompt, verbatim from VOICE.md. */
  voice: string
  lore: Lore
}

export function loadNarrativeVoice(options: { voicePath: string; lorePath: string }): NarrativeVoice {
  return {
    voice: readFileSync(options.voicePath, 'utf8').trim(),
    lore: parseLore(readFileSync(options.lorePath, 'utf8')),
  }
}

/**
 * LORE.md is a handful of `## ` sections of bullet lines. The prose under
 * the title and above the first section is the premise. A latent line is
 * `- id | trigger: condition | reveal`; a latent line that does not parse
 * is dropped rather than half-read, because a trigger that cannot be
 * evaluated would either never fire or always fire.
 */
export function parseLore(text: string): Lore {
  const sections = new Map<string, string[]>()
  let current = 'premise'
  const premise: string[] = []
  for (const raw of text.split('\n')) {
    const line = raw.trimEnd()
    const heading = /^##\s+(.+)$/.exec(line)
    if (heading) {
      current = heading[1]!.trim().toLowerCase()
      sections.set(current, [])
      continue
    }
    if (current === 'premise') {
      if (!line.startsWith('# ')) premise.push(line)
      continue
    }
    if (line.startsWith('- ')) sections.get(current)!.push(line.slice(2).trim())
  }
  const latent = (sections.get('latent') ?? []).flatMap((line) => {
    const parts = line.split('|').map((part) => part.trim())
    if (parts.length < 3) return []
    const trigger = parts[1]!.replace(/^trigger:\s*/i, '')
    if (!parts[0] || !trigger || !parts[2]) return []
    return [{ id: parts[0], trigger, reveal: parts.slice(2).join(' | ') }]
  })
  return {
    premise: premise.join('\n').trim(),
    goals: sections.get('goals') ?? [],
    cast: sections.get('cast') ?? [],
    runningBits: sections.get('running bits') ?? [],
    latent,
  }
}

/** What teeny is told: the goals, the world, the bits, and only the lore that has already come out. */
export function renderLoreForTeeny(lore: Lore, surfaced: string[]): string {
  const known = lore.latent.filter((item) => surfaced.includes(item.id))
  return [
    'the goals, in your own words every time:',
    ...lore.goals.map((goal) => `- ${goal}`),
    '',
    'who is around:',
    ...lore.cast.map((line) => `- ${line}`),
    '',
    'the bits:',
    ...lore.runningBits.map((line) => `- ${line}`),
    ...(known.length > 0 ? ['', 'what has come out so far:', ...known.map((item) => `- ${item.reveal}`)] : []),
  ].join('\n')
}

/**
 * What the producer is told: everything, with the triggers, and which of
 * them the world state has tripped today. The producer never evaluates a
 * trigger itself; the pipeline does, deterministically, and hands over the
 * list.
 */
export function renderLoreForProducer(lore: Lore, surfaced: string[], live: string[]): string {
  return [
    'the premise:',
    lore.premise,
    '',
    'the goals (teeny restates them every episode; you keep the story pointed at them):',
    ...lore.goals.map((goal) => `- ${goal}`),
    '',
    'the cast:',
    ...lore.cast.map((line) => `- ${line}`),
    '',
    'the running bits:',
    ...lore.runningBits.map((line) => `- ${line}`),
    '',
    'latent lore. surfaced items are part of the story for good. an item marked LIVE has its',
    'trigger true today and may come out now, sideways, never announced. anything else stays dark:',
    ...lore.latent.map((item) => {
      const status = surfaced.includes(item.id) ? 'surfaced' : live.includes(item.id) ? 'LIVE' : 'dark'
      return `- ${item.id} [${status}] (trigger: ${item.trigger}) ${status === 'dark' ? '' : item.reveal}`.trimEnd()
    }),
  ].join('\n')
}

/**
 * A trigger is a small boolean over the world state's fields, and no more:
 * `field op number`, `field between a and b`, a bare boolean field, joined
 * by `and`. Anything the grammar does not cover is false, so a typo in the
 * bible keeps an item dark rather than letting it out.
 */
export function evaluateTrigger(trigger: string, fields: Record<string, number | boolean | undefined>): boolean {
  // `between a and b` is one clause; its `and` is not a conjunction.
  return trigger
    .replace(/between\s+(-?[\d.]+)\s+and\s+(-?[\d.]+)/gi, 'between $1..$2')
    .split(/\s+and\s+/i)
    .map((clause) => clause.trim())
    .every((clause) => evaluateClause(clause, fields))
}

function evaluateClause(clause: string, fields: Record<string, number | boolean | undefined>): boolean {
  const between = /^(\w+)\s+between\s+(-?[\d.]+)\.\.(-?[\d.]+)$/i.exec(clause)
  if (between) {
    const value = fields[between[1]!]
    return typeof value === 'number' && value >= Number(between[2]) && value <= Number(between[3])
  }
  const compare = /^(\w+)\s*(<=|>=|<|>|==|=)\s*(-?[\d.]+)$/.exec(clause)
  if (compare) {
    const value = fields[compare[1]!]
    if (typeof value !== 'number') return false
    const target = Number(compare[3])
    switch (compare[2]) {
      case '<=':
        return value <= target
      case '>=':
        return value >= target
      case '<':
        return value < target
      case '>':
        return value > target
      default:
        return value === target
    }
  }
  const bare = /^(\w+)$/.exec(clause)
  if (bare) return fields[bare[1]!] === true
  return false
}

/** The latent items whose triggers the given fields satisfy and that have not surfaced yet. */
export function liveLatent(lore: Lore, surfaced: string[], fields: Record<string, number | boolean | undefined>): string[] {
  return lore.latent
    .filter((item) => !surfaced.includes(item.id) && evaluateTrigger(item.trigger, fields))
    .map((item) => item.id)
}
