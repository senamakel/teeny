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
    expect(prompt).not.toContain('x_critique')
    expect(systemPrompt({ botUsername: 'stonksrocks', dryRun: false, critic: true })).toContain('call\n  x_critique')

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
      // The rule is unchanged; the wording is not. "Never shill" was dropped
      // because the account had started using the word ABOUT people -- it
      // called somebody offering it its own token a shill -- and a prompt that
      // repeats a word teaches it as vocabulary.
      expect(prompt).toMatch(/[Nn]ever endorse/)
      expect(prompt).toMatch(/[Nn]ever use the word shill|not moving your own bag/)
    }
  })
})

describe('the drafter is not weaker than the task agent', () => {
  it('carries the same tone rules in both prompts', () => {
    const options = { botUsername: 'TinyMascot', dryRun: false }
    for (const prompt of [systemPrompt(options), draftPrompt(options)]) {
      expect(prompt).toContain('roast back')
      expect(prompt).toContain('do not roast and')
      expect(prompt).toMatch(/Reach for a GIF often/)
      // The default has moved: warmth first, roasting as the exception. Both
      // prompts must agree, or the cron drafter -- the one that actually
      // answers people -- is the one running on the older, colder rules.
      expect(prompt).toContain('Build people up')
      expect(prompt).toContain('Do not deflate people')
    }
  })
})
