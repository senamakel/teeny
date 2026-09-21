import { mkdirSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import type { Thought } from './distill.js'
import type { GmPost } from './gm.js'
import { renderTurn, type Craziness, type RunStats } from './evaluate.js'
import { renderVariants, type Variants } from './variants.js'
import type { BackroomsRun } from './types.js'

/**
 * A run on disk, twice: the whole record as JSON, for replay and for the
 * cut later, and a markdown transcript for reading. Nothing here is a
 * post; the directory is a notebook.
 */
export class FileBackroomsStore {
  constructor(private readonly dir: string) {}

  save(run: BackroomsRun, extras: SavedExtras = {}): { json: string; markdown: string } {
    mkdirSync(this.dir, { recursive: true })
    const json = join(this.dir, `${run.id}.json`)
    const markdown = join(this.dir, `${run.id}.md`)
    writeFileSync(json, JSON.stringify({ ...run, ...extras }, null, 2))
    writeFileSync(markdown, renderMarkdown(run, extras))
    return { json, markdown }
  }
}

export interface SavedExtras {
  stats?: RunStats
  judge?: Craziness
  thought?: Thought
  variants?: Variants
  gm?: GmPost
}

export function renderMarkdown(run: BackroomsRun, extras: SavedExtras): string {
  const lines = [
    `# backrooms ${run.id}`,
    '',
    `seed (${run.seed.kind}${run.seed.source ? `, ${run.seed.source}` : ''}): ${run.seed.title ? `**${run.seed.title}** — ` : ''}${run.seed.text}`,
    '',
    `agents: ${run.agents.map((agent) => `${agent.agent.id} @ ${run.settings.temps[agent.agent.id] ?? agent.agent.temperature} (${agent.tokens} tok; ${agent.blocks.map((block) => block.kind).join(', ')})`).join(' · ')}`,
    `settings: rng ${run.settings.rng}, ${run.settings.turns} turns, budget ${run.settings.budget}, drift every ${run.settings.driftEvery}, ${run.settings.maxToolCalls} tool calls/turn`,
    ...run.agents.flatMap((agent) => agent.problems.map((problem) => `problem (${agent.agent.id}): ${problem}`)),
    '',
  ]
  if (extras.thought) {
    lines.push(
      '## the thought',
      '',
      `*${extras.thought.premise}*`,
      '',
      ...(extras.thought.wire ? [`the wire: ${extras.thought.wire}`, ''] : []),
      extras.thought.thought,
      '',
      '**as a post**',
      '',
      ...extras.thought.post.split('\n').map((line) => `> ${line}`),
      '',
      extras.thought.objects.length > 0 ? `objects: ${extras.thought.objects.join(' · ')}` : '',
      '',
    )
  }
  if (extras.gm) {
    lines.push(
      '## the good morning',
      '',
      ...extras.gm.text.split('\n').map((line) => `> ${line}`),
      '',
      extras.gm.wire ? `the wire: ${extras.gm.wire}` : 'the wire: (not in the post)',
      extras.gm.continues ? `continues: ${extras.gm.continues}` : '',
      extras.gm.reasons,
      '',
    )
  }
  if (extras.variants) lines.push(...renderVariants(extras.variants))
  lines.push('## transcript', '', ...run.turns.map((turn, index) => renderTurn(turn, index)))
  if (extras.stats) {
    lines.push('## stats', '', '```json', JSON.stringify(extras.stats, null, 2), '```', '')
  }
  if (extras.judge) {
    const total = Object.values(extras.judge.scores).reduce((a, b) => a + b, 0)
    lines.push(
      '## judge',
      '',
      `total ${total}/21 — ${Object.entries(extras.judge.scores)
        .map(([key, value]) => `${key} ${value}`)
        .join(', ')}`,
      '',
      extras.judge.reasons,
      '',
      ...extras.judge.quotables.map((line) => `> ${line}`),
      '',
      extras.judge.deadSpots.length > 0 ? `dead spots: ${extras.judge.deadSpots.join(', ')}` : 'no dead spots',
      '',
      `verdict: ${extras.judge.verdict}`,
      '',
    )
  }
  return lines.join('\n')
}

export function runId(now: Date, slug: string): string {
  const stamp = now.toISOString().replace(/[-:]/g, '').replace(/T(\d{4})\d{2}\.\d+Z$/, '-$1')
  const clean = slug
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 40)
  return clean ? `${stamp}-${clean}` : stamp
}
