import { existsSync, mkdtempSync, readdirSync, readFileSync, utimesSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { describe, expect, it, vi } from 'vitest'
import { loadPersona, personaText, renderPersona } from '../src/agent/soul.js'
import { InMemoryPostLedger } from '../src/cadence/ledger.js'
import { InMemoryPeople } from '../src/cadence/people.js'
import { MemoryJournal } from '../src/journal/journal.js'
import { fit, LEARNINGS_SECTIONS, reflect } from '../src/learnings/reflect.js'

const now = Date.parse('2026-10-05T05:00:00Z')

function brief(lines: Record<string, string[]> = {}): string {
  return [
    '# Learnings',
    ...LEARNINGS_SECTIONS.flatMap((section) => ['', `## ${section}`, ...(lines[section] ?? ['- something true'])]),
  ].join('\n')
}

function workspace(): string {
  return mkdtempSync(join(tmpdir(), 'learnings-'))
}

async function record() {
  const ledger = new InMemoryPostLedger()
  await ledger.record({ postId: 'p1', kind: 'chart', postedAt: '2026-10-04T15:00:00Z', text: 'the chart' })
  await ledger.sample('p1', { at: '2026-10-04T18:00:00Z', impressions: 900, likes: 40, replies: 6, reposts: 3, quotes: 0 })
  const journal = new MemoryJournal()
  await journal.record({
    at: '2026-10-04T16:00:00Z',
    trigger: 'mention',
    incoming: '@teeny is the serial back?',
    tone: 'friendly',
    recalled: [],
    tools: [],
    thinking: '',
    said: 'tuesday, promise',
    declined: [],
    outcome: 'posted',
  })
  const people = new InMemoryPeople()
  await people.observe({ userId: '7', username: 'ana', kind: 'reply', at: '2026-10-04T16:00:00Z' })
  return { ledger, journal, people }
}

describe('reflect', () => {
  it('folds the record into the previous brief and keeps yesterday\'s copy', async () => {
    const root = workspace()
    const path = join(root, 'memory', 'LEARNINGS.md')
    await reflect({ path, text: { complete: vi.fn().mockResolvedValue(brief()) }, botUsername: 'teeny', days: 7, maxChars: 10_000, now: () => now - 86_400_000 })

    const complete = vi.fn().mockResolvedValue(brief({ 'People I know': ['- @ana: asked when the serial is back'] }))
    const recall = vi.fn().mockResolvedValue('a note')
    const result = await reflect({
      path,
      text: { complete },
      ...(await record()),
      memory: { recall, captureActivity: vi.fn().mockResolvedValue(undefined) },
      botUsername: 'teeny',
      days: 7,
      maxChars: 10_000,
      now: () => now,
    })

    expect(result.outcome).toBe('written')
    const { system, prompt } = complete.mock.calls[0]![0] as { system: string; prompt: string }
    // An update, not a fresh write: the old brief is in the prompt and the
    // system prompt says to keep what is still true.
    expect(system).toContain('Keep what is still true from the previous brief')
    expect(prompt).toContain('THE PREVIOUS BRIEF:\n# Learnings')
    expect(prompt).toContain('[2026-10-04 chart] (900 views, 40 likes, 6 replies, 3 reposts): the chart')
    expect(prompt).toContain('@teeny is the serial back? → you: tuesday, promise')
    expect(prompt).toContain('@ana: score 3')
    expect(readFileSync(path, 'utf8')).toContain('@ana: asked when the serial is back')
    expect(readdirSync(join(root, 'memory', 'history'))).toHaveLength(1)
    expect(existsSync(`${path}.tmp`)).toBe(false)
  })

  it('keeps the previous brief when the draft is not a brief', async () => {
    const root = workspace()
    const path = join(root, 'LEARNINGS.md')
    writeFileSync(path, brief())

    const result = await reflect({
      path,
      text: { complete: vi.fn().mockResolvedValue('Sure! Here is an essay about my week.') },
      botUsername: 'teeny',
      days: 7,
      maxChars: 10_000,
      now: () => now,
    })

    expect(result.outcome).toBe('kept')
    expect(readFileSync(path, 'utf8')).toBe(brief())
  })

  it('writes the first brief from nothing, and survives every source being down', async () => {
    const path = join(workspace(), 'LEARNINGS.md')
    const down = () => Promise.reject(new Error('down'))

    const result = await reflect({
      path,
      text: { complete: vi.fn().mockResolvedValue('```markdown\n' + brief() + '\n```') },
      ledger: { since: down },
      journal: { recent: down },
      people: { top: down },
      memory: { recall: down, captureActivity: down },
      botUsername: 'teeny',
      days: 7,
      maxChars: 10_000,
      now: () => now,
    })

    expect(result.outcome).toBe('written')
    expect(result.problems.length).toBeGreaterThanOrEqual(4)
    expect(readFileSync(path, 'utf8').startsWith('# Learnings')).toBe(true)
  })
})

describe('fit', () => {
  it('trims the longest section first, so every heading survives', () => {
    const long = brief({ 'People I know': Array.from({ length: 200 }, (_, index) => `- @person${index}: a regular`) })
    const fitted = fit(long, 1_500)

    expect(fitted.length).toBeLessThanOrEqual(1_500)
    for (const section of LEARNINGS_SECTIONS) expect(fitted).toContain(`## ${section}`)
  })
})

describe('the learnings in the persona', () => {
  it('reaches the rendered persona, and a rewrite reaches it without a reload', () => {
    const root = workspace()
    const path = join(root, 'LEARNINGS.md')
    writeFileSync(join(root, 'SOUL.md'), 'You are Tiny.')
    const persona = loadPersona({ soulPath: join(root, 'SOUL.md'), learningsPath: path })
    const live = () => renderPersona(persona)

    expect(persona.learnings).toBeUndefined()
    expect(personaText(live)).not.toContain('<learnings>')

    writeFileSync(path, '# Learnings\n\n- @ana likes the lab lore')
    expect(personaText(live)).toContain('<learnings>\n# Learnings\n\n- @ana likes the lab lore\n</learnings>')

    writeFileSync(path, '# Learnings\n\n- @ana moved to the serial')
    // A rewrite inside the same mtime tick would be missed by mtime alone;
    // the size check catches most, and a forced mtime catches the rest.
    utimesSync(path, new Date(), new Date(Date.now() + 5_000))
    expect(personaText(live)).toContain('@ana moved to the serial')
  })
})
