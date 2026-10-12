// Golden generator for teeny-agent's prompt parity test: runs the legacy
// prompt builders over fixed options and writes what they returned.
//   (cd legacy && ./node_modules/.bin/tsx ../crates/teeny-agent/fixtures/prompts/generate.mts)
// The persona carries no skills: the Rust port no longer pastes them in.
import { writeFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
const L = fileURLToPath(new URL('../../../../legacy/src', import.meta.url))
const OUT = fileURLToPath(new URL('.', import.meta.url))
const { systemPrompt, draftPrompt, chatPrompt, contentPrompt } = await import(`${L}/agent/prompt.ts`)
const { chartPrompt } = await import(`${L}/agent/chart-prompt.ts`)

const persona = { soul: '# SOUL.md - Tiny\n\nYou are Tiny.', context: '# CONTEXT.md\n\n$TINY is the only token.', skills: [] }
// Two cases flip every toggle both ways: no persona, no critic, live; and
// persona, critic, dry run.
const cases = []
for (const [withPersona, critic, dryRun] of [[false, false, false], [true, true, true]]) {
  {
    {
      const options = { botUsername: '@teenydroid', dryRun, critic, ...(withPersona ? { persona } : {}) }
      cases.push({
        persona: withPersona,
        critic,
        dryRun,
        system: systemPrompt(options),
        draft: draftPrompt(options),
        chat: chatPrompt({ ...options, channelHandle: 'teenythebot' }),
        contentX: contentPrompt({ ...options, channels: ['x'] }),
        contentBoth: contentPrompt({ ...options, channels: ['x', 'telegram'] }),
        contentTelegram: contentPrompt({ ...options, channels: ['telegram'] }),
        chart: chartPrompt({ ...options, botUsername: 'teenydroid', symbol: 'TINY' }),
      })
    }
  }
}
writeFileSync(`${OUT}/prompts.json`, JSON.stringify(cases, null, 2) + '\n')
console.log(`wrote ${cases.length} cases`)
