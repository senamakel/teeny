import { mkdtempSync, mkdirSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { describe, expect, it } from 'vitest'
import { loadPersona, renderPersona } from '../src/agent/soul.js'
import { systemPrompt, draftPrompt } from '../src/agent/prompt.js'

function fixture(): string {
  const root = mkdtempSync(join(tmpdir(), 'persona-'))
  writeFileSync(join(root, 'SOUL.md'), '# SOUL.md - Tiny\n\nYou are Tiny.')
  writeFileSync(join(root, 'CONTEXT.md'), '# CONTEXT.md\n\n$TINY is the only token.')
  mkdirSync(join(root, 'skills', 'humanizer'), { recursive: true })
  writeFileSync(join(root, 'skills', 'humanizer', 'SKILL.md'), '# Humanizer\n\nNo em dashes.')
  writeFileSync(join(root, 'skills', 'brevity.md'), '# Brevity\n\nBe short.')
  return root
}

describe('loadPersona', () => {
  it('reads SOUL.md and both skill layouts', () => {
    const root = fixture()
    const persona = loadPersona({
      soulPath: join(root, 'SOUL.md'),
      contextPath: join(root, 'CONTEXT.md'),
      skillsDir: join(root, 'skills'),
    })

    expect(persona.soul).toContain('You are Tiny.')
    expect(persona.context).toContain('$TINY is the only token.')
    expect(persona.skills.map((skill) => skill.name)).toEqual(['brevity', 'humanizer'])
  })

  it('is a no-op when the files do not exist', () => {
    const persona = loadPersona({
      soulPath: join(tmpdir(), 'missing-soul.md'),
      contextPath: join(tmpdir(), 'missing-context.md'),
      skillsDir: join(tmpdir(), 'missing-skills'),
    })

    expect(persona.context).toBeUndefined()
    expect(persona.soul).toBeUndefined()
    expect(persona.skills).toEqual([])
  })
})

describe('prompts', () => {
  it('puts the soul and every skill into both prompts', () => {
    const root = fixture()
    const persona = loadPersona({
      soulPath: join(root, 'SOUL.md'),
      contextPath: join(root, 'CONTEXT.md'),
      skillsDir: join(root, 'skills'),
    })
    const options = { botUsername: 'TinyHumanAI', dryRun: true, persona }

    for (const prompt of [systemPrompt(options), draftPrompt(options)]) {
      expect(prompt).toContain('You are Tiny.')
      expect(prompt).toContain('$TINY is the only token.')
      expect(prompt).toContain('<skill name="humanizer">')
      expect(prompt).toContain('No em dashes.')
      expect(prompt).toContain('<skill name="brevity">')
    }
  })

  it('falls back to the generic identity without a soul', () => {
    const prompt = systemPrompt({ botUsername: 'stonksrocks', dryRun: false })

    expect(prompt).toContain('@stonksrocks')
    expect(prompt).toContain('DRY RUN is off')
  })

  it('renders nothing for an empty persona', () => {
    expect(renderPersona({ skills: [] })).toBe('')
  })
})

describe('the one commercial rule', () => {
  it('states it in both prompts even without a context file', () => {
    for (const prompt of [
      systemPrompt({ botUsername: 'TinyMascot', dryRun: true }),
      draftPrompt({ botUsername: 'TinyMascot', dryRun: true }),
    ]) {
      expect(prompt).toContain('$TINY is the only token')
      expect(prompt).toMatch(/[Nn]ever shill/)
    }
  })
})

describe('the drafter is not weaker than the task agent', () => {
  it('carries the same tone rules in both prompts', () => {
    const options = { botUsername: 'TinyMascot', dryRun: false }
    for (const prompt of [systemPrompt(options), draftPrompt(options)]) {
      expect(prompt).toContain('roast them back')
      expect(prompt).toContain('fair skepticism')
      expect(prompt).toContain('do not roast and')
      expect(prompt).toMatch(/Reach for a GIF often/)
    }
  })
})
