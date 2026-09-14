import { open, readdir, readFile, stat } from 'node:fs/promises'
import { basename, join } from 'node:path'
import { parse as parseToml } from 'smol-toml'
import { recordQuietly, sha256, type InferenceLedger } from './ledger.js'

/**
 * The brain, read from its checkout.
 *
 * Teeny's brain is an OpenCompany company whose whole state is a git working
 * tree: the definition under `company/<slug>/`, the runtime record under
 * `companies/<slug>/`. On the box that tree is mounted into this process
 * read-only, and this is the one place that reads it. Two things come out:
 * the brain's token spend, folded into the ledger so the counter on the page
 * counts both minds, and a snapshot of the roster, the workflows and the
 * latest runs, served so the page can show the agents talking to each other.
 *
 * Nothing here writes to the tree, and nothing here reads a file the tree's
 * own `.gitignore` keeps out of git, with one exception: `usage.jsonl`, which
 * carries token counts and nothing else, and is folded into totals rather
 * than served.
 */

export interface BrainAgent {
  id: string
  role: string
  description: string
  tier?: string
}

export interface BrainNode {
  id: string
  kind: string
  name: string
  summary?: string
  agent?: string
  schedule?: string
}

export interface BrainEdge {
  from: string
  to: string
  label?: string
}

export interface BrainWorkflow {
  id: string
  name: string
  description?: string
  nodes: BrainNode[]
  edges: BrainEdge[]
}

/** One task attempt, as the host records it. Field names are the host's. */
export interface BrainRun {
  id: string
  agentId: string
  status: string
  workflowRunId?: string
  nodeId?: string
  createdAtMillis: number
  startedAtMillis?: number
  finishedAtMillis?: number
  usage?: { input?: number; output?: number; cachedInput?: number }
  stepCount?: number
  error?: string
}

export interface BrainStep {
  runId: string
  stepSeq: number
  atMillis: number
  step: { kind: string; status: string; label: string; detail?: string; result?: string }
}

export interface BrainSnapshot {
  source: 'live'
  generatedAt: string
  company: { name: string; output?: string; slug: string }
  agents: BrainAgent[]
  workflows: BrainWorkflow[]
  runs: BrainRun[]
  steps: BrainStep[]
}

/** One line of `usage.jsonl`: a metered model call. camelCase, as the host writes it. */
interface UsageSample {
  atMillis: number
  agent: string
  provider?: string
  model?: string
  inputTokens?: number
  outputTokens?: number
  cachedInputTokens?: number
  kind?: string
}

/** The last `bytes` of a file, as whole lines. A missing file is no lines. */
export async function tailLines(path: string, bytes = 512 * 1024): Promise<string[]> {
  let handle
  try {
    handle = await open(path, 'r')
  } catch {
    return []
  }
  try {
    const { size } = await handle.stat()
    const start = Math.max(0, size - bytes)
    const buffer = Buffer.alloc(size - start)
    await handle.read(buffer, 0, buffer.length, start)
    const text = buffer.toString('utf8')
    // A cut that landed mid-line leaves a fragment at the front; drop it.
    const lines = text.split('\n')
    if (start > 0) lines.shift()
    return lines.filter((line) => line.trim().length > 0)
  } finally {
    await handle.close()
  }
}

function jsonLines<T>(lines: string[]): T[] {
  const out: T[] = []
  for (const line of lines) {
    try {
      out.push(JSON.parse(line) as T)
    } catch {
      // A torn write at the tail is the host mid-sentence, not a broken record.
    }
  }
  return out
}

function str(value: unknown, fallback = ''): string {
  return typeof value === 'string' ? value : fallback
}

export class BrainReader {
  private readonly dir: string
  private readonly slug: string
  /** Where the fold got to. In-process: the ledger dedupes by id, so a restart re-reads harmlessly. */
  private foldedTo = 0

  constructor(options: { dir: string; slug?: string }) {
    this.dir = options.dir
    this.slug = options.slug ?? 'teeny'
  }

  private definition(...parts: string[]): string {
    return join(this.dir, 'company', this.slug, ...parts)
  }

  private record(...parts: string[]): string {
    return join(this.dir, 'companies', this.slug, ...parts)
  }

  /** Whether there is a brain at the path at all. */
  async exists(): Promise<boolean> {
    try {
      return (await stat(this.definition('company.toml'))).isFile()
    } catch {
      return false
    }
  }

  /**
   * Folds every usage sample newer than the last fold into the ledger.
   * Returns how many rows were offered; the ledger drops the ones it has.
   */
  async foldUsage(ledger: InferenceLedger): Promise<number> {
    const lines = await tailLines(this.record('usage.jsonl'), 4 * 1024 * 1024)
    let offered = 0
    let newest = this.foldedTo
    for (const line of lines) {
      let sample: UsageSample
      try {
        sample = JSON.parse(line) as UsageSample
      } catch {
        continue
      }
      if (typeof sample.atMillis !== 'number' || sample.atMillis < this.foldedTo) continue
      const input = sample.inputTokens ?? 0
      const output = sample.outputTokens ?? 0
      await recordQuietly(ledger, {
        id: `b-${sample.atMillis}-${sha256(line).slice(0, 12)}`,
        at: new Date(sample.atMillis).toISOString(),
        lane: 'brain',
        source: str(sample.agent, 'company'),
        ...(sample.kind ? { kind: sample.kind } : {}),
        model: str(sample.model) || str(sample.provider, 'unknown'),
        tokens: {
          input,
          output,
          reasoning: 0,
          cached: sample.cachedInputTokens ?? 0,
          total: input + output,
        },
      })
      offered += 1
      if (sample.atMillis > newest) newest = sample.atMillis
    }
    // Samples in the same millisecond as the newest could still arrive, so
    // the cursor stays one behind; the ledger's id check makes the overlap free.
    this.foldedTo = newest
    return offered
  }

  private async agents(): Promise<BrainAgent[]> {
    const dir = this.definition('agents')
    let files: string[]
    try {
      files = (await readdir(dir)).filter((name) => name.endsWith('.toml')).sort()
    } catch {
      return []
    }
    const agents: BrainAgent[] = []
    for (const file of files) {
      try {
        const doc = parseToml(await readFile(join(dir, file), 'utf8')) as Record<string, unknown>
        const id = basename(file, '.toml')
        agents.push({
          id,
          role: str(doc.role, id),
          description: str(doc.description),
          ...(typeof doc.tier === 'string' ? { tier: doc.tier } : {}),
        })
      } catch {
        // A file the curator is mid-edit on is not a roster change yet.
      }
    }
    return agents
  }

  private async workflows(): Promise<BrainWorkflow[]> {
    const dir = this.definition('workflows')
    let files: string[]
    try {
      files = (await readdir(dir)).filter((name) => name.endsWith('.toml')).sort()
    } catch {
      return []
    }
    const workflows: BrainWorkflow[] = []
    for (const file of files) {
      try {
        const doc = parseToml(await readFile(join(dir, file), 'utf8')) as Record<string, unknown>
        const nodes = (Array.isArray(doc.node) ? doc.node : []) as Array<Record<string, unknown>>
        const edges = (Array.isArray(doc.edge) ? doc.edge : []) as Array<Record<string, unknown>>
        workflows.push({
          id: str(doc.id, basename(file, '.toml')),
          name: str(doc.name, str(doc.id, basename(file, '.toml'))),
          ...(typeof doc.description === 'string' ? { description: doc.description } : {}),
          nodes: nodes.map((node) => ({
            id: str(node.id),
            kind: str(node.kind, 'agent'),
            name: str(node.name, str(node.id)),
            ...(typeof node.summary === 'string' ? { summary: node.summary } : {}),
            ...(typeof node.agent === 'string' ? { agent: node.agent } : {}),
            ...(typeof node.schedule === 'string' ? { schedule: node.schedule } : {}),
          })),
          edges: edges.map((edge) => ({
            from: str(edge.from),
            to: str(edge.to),
            ...(typeof edge.label === 'string' ? { label: edge.label } : {}),
          })),
        })
      } catch {
        // Same: a half-written workflow is not one yet.
      }
    }
    return workflows
  }

  private async company(): Promise<BrainSnapshot['company']> {
    try {
      const doc = parseToml(await readFile(this.definition('company.toml'), 'utf8')) as {
        company?: { name?: string; output?: string }
      }
      return {
        name: str(doc.company?.name, this.slug),
        ...(typeof doc.company?.output === 'string' ? { output: doc.company.output } : {}),
        slug: this.slug,
      }
    } catch {
      return { name: this.slug, slug: this.slug }
    }
  }

  /**
   * The latest runs, newest first, one row per run. The host rewrites a
   * run's row as it moves, so a file may carry a run several times; the last
   * occurrence is the current one.
   */
  private async runs(limit: number): Promise<BrainRun[]> {
    const rows = jsonLines<BrainRun>(await tailLines(this.record('runs.jsonl')))
    const byId = new Map<string, BrainRun>()
    for (const row of rows) if (row && typeof row.id === 'string') byId.set(row.id, row)
    return [...byId.values()]
      .sort((a, b) => (b.createdAtMillis ?? 0) - (a.createdAtMillis ?? 0))
      .slice(0, limit)
  }

  private async steps(runIds: Set<string>, limit: number): Promise<BrainStep[]> {
    const rows = jsonLines<BrainStep>(await tailLines(this.record('run-steps.jsonl'), 2 * 1024 * 1024))
    return rows
      .filter((row) => row && runIds.has(row.runId) && row.step)
      .map((row) => ({
        runId: row.runId,
        stepSeq: row.stepSeq,
        atMillis: row.atMillis,
        step: {
          kind: str(row.step.kind, 'note'),
          status: str(row.step.status, 'ok'),
          label: str(row.step.label),
          ...(typeof row.step.detail === 'string' ? { detail: row.step.detail.slice(0, 600) } : {}),
          ...(typeof row.step.result === 'string' ? { result: row.step.result.slice(0, 600) } : {}),
        },
      }))
      .sort((a, b) => a.atMillis - b.atMillis)
      .slice(-limit)
  }

  async snapshot(options: { runs?: number; steps?: number } = {}): Promise<BrainSnapshot> {
    const [company, agents, workflows, runs] = await Promise.all([
      this.company(),
      this.agents(),
      this.workflows(),
      this.runs(options.runs ?? 40),
    ])
    const steps = await this.steps(new Set(runs.map((run) => run.id)), options.steps ?? 300)
    return { source: 'live', generatedAt: new Date().toISOString(), company, agents, workflows, runs, steps }
  }
}
