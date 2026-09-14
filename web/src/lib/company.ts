/**
 * The brain: Teeny's mind as an OpenCompany company, read from its checkout
 * by the bot and served at /brain, and its public repository on GitHub.
 *
 * The definition (roster, workflows) and the record (runs, steps) below are
 * the real ones from `teenydroid/brain` at the time of writing, so a page
 * with no bot behind it still shows the true shape of the thing. The sample
 * run is the one thing invented here, and the page says so.
 */

import { readApi } from './api'

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
  source: 'live' | 'sample'
  generatedAt: string
  company: { name: string; output?: string; slug: string }
  agents: BrainAgent[]
  workflows: BrainWorkflow[]
  runs: BrainRun[]
  steps: BrainStep[]
}

export const BRAIN_REPO = process.env.BRAIN_REPO ?? 'teenydroid/brain'
export const BRAIN_REPO_URL = `https://github.com/${BRAIN_REPO}`
export const OPENCOMPANY_URL = 'https://github.com/tinyhumansai/opencompany'

export const SAMPLE_AGENTS: BrainAgent[] = [
  {
    id: 'orchestrator',
    role: 'Orchestrator',
    description: 'Decides what the brain works on next, routes it to the right teammate, and keeps the board honest.',
    tier: 'orchestrator',
  },
  {
    id: 'scout',
    role: 'Scout',
    description: 'Watches what is happening around Teeny — $TINY, TinyHumans, the builders it cheers on — and records it as signals, never as conclusions.',
  },
  {
    id: 'scribe',
    role: 'Scribe',
    description: 'Turns what happened into what was learned: reads runs and signals, writes learnings and notes, keeps the record short and true.',
  },
  {
    id: 'curator',
    role: 'Curator',
    description: 'The self-editor. Revises goals, tasks, prompts, skills and the roster under /data/company/teeny, and records every change.',
  },
]

export const SAMPLE_WORKFLOW: BrainWorkflow = {
  id: 'reflect',
  name: 'Reflect',
  description: 'Look at what happened, write down what it taught, and revise the definition where the record says to.',
  nodes: [
    { id: 'daily', kind: 'trigger', name: 'Every morning', summary: 'Fires daily at 06:00 UTC, and whenever the orchestrator runs it by hand.', schedule: '0 6 * * *' },
    { id: 'look', kind: 'agent', name: 'Look outward and at the record', summary: 'Record what happened since the last pass as signals. Observations only.', agent: 'scout' },
    { id: 'learn', kind: 'agent', name: 'Write what it taught', summary: 'Fold the new signals and the settled runs into learnings; update Goals.md to match the goals ledger.', agent: 'scribe' },
    { id: 'gate', kind: 'condition', name: 'Does the record call for a change?', summary: 'Branch on whether a learning names something in the definition that is now wrong.' },
    { id: 'revise', kind: 'agent', name: 'Revise the definition', summary: 'Make the smallest change the record justifies, check it, record it, request a reload.', agent: 'curator' },
    { id: 'report', kind: 'output', name: 'Report to the operator', summary: 'What was seen, what was learned, what changed — three short lists.' },
  ],
  edges: [
    { from: 'daily', to: 'look' },
    { from: 'look', to: 'learn' },
    { from: 'learn', to: 'gate' },
    { from: 'gate', to: 'revise', label: 'yes' },
    { from: 'gate', to: 'report', label: 'no' },
    { from: 'revise', to: 'report' },
  ],
}

const T0 = Date.parse('2026-09-14T06:00:00Z')

const SAMPLE_RUNS: BrainRun[] = [
  { id: 'run-look', agentId: 'scout', status: 'succeeded', workflowRunId: 'wf-0914', nodeId: 'look', createdAtMillis: T0, startedAtMillis: T0 + 2_000, finishedAtMillis: T0 + 4 * 60_000, usage: { input: 96_400, output: 6_120, cachedInput: 41_000 }, stepCount: 7 },
  { id: 'run-learn', agentId: 'scribe', status: 'succeeded', workflowRunId: 'wf-0914', nodeId: 'learn', createdAtMillis: T0 + 4 * 60_000, startedAtMillis: T0 + 4 * 60_000 + 1_500, finishedAtMillis: T0 + 11 * 60_000, usage: { input: 128_200, output: 9_480, cachedInput: 60_100 }, stepCount: 9 },
  { id: 'run-revise', agentId: 'curator', status: 'succeeded', workflowRunId: 'wf-0914', nodeId: 'revise', createdAtMillis: T0 + 11 * 60_000, startedAtMillis: T0 + 11 * 60_000 + 1_000, finishedAtMillis: T0 + 19 * 60_000, usage: { input: 141_900, output: 11_260, cachedInput: 70_400 }, stepCount: 11 },
]

const SAMPLE_STEPS: BrainStep[] = [
  { runId: 'run-look', stepSeq: 1, atMillis: T0 + 5_000, step: { kind: 'thinking', status: 'ok', label: 'Reading the signals ledger since the last pass' } },
  { runId: 'run-look', stepSeq: 2, atMillis: T0 + 40_000, step: { kind: 'tool_call', status: 'ok', label: 'search', detail: '$TINY OR TinyHumans, last 24h', result: '14 posts, 2 threads worth a signal' } },
  { runId: 'run-look', stepSeq: 3, atMillis: T0 + 95_000, step: { kind: 'tool_call', status: 'ok', label: 'ledger.record', detail: 'signals: "the chart post drew a real question about the level"' } },
  { runId: 'run-look', stepSeq: 4, atMillis: T0 + 150_000, step: { kind: 'tool_call', status: 'ok', label: 'ledger.record', detail: 'signals: "openhuman tagged a release; nobody in the replies mentioned it"' } },
  { runId: 'run-look', stepSeq: 5, atMillis: T0 + 220_000, step: { kind: 'note', status: 'ok', label: 'Two signals recorded. Nothing decided.' } },
  { runId: 'run-learn', stepSeq: 1, atMillis: T0 + 4 * 60_000 + 6_000, step: { kind: 'thinking', status: 'ok', label: 'Folding the new signals against the settled runs' } },
  { runId: 'run-learn', stepSeq: 2, atMillis: T0 + 5 * 60_000, step: { kind: 'tool_call', status: 'ok', label: 'ledger.read', detail: 'learnings, last 7 days', result: '3 rows' } },
  { runId: 'run-learn', stepSeq: 3, atMillis: T0 + 7 * 60_000, step: { kind: 'tool_call', status: 'ok', label: 'ledger.record', detail: 'learnings: "a release nobody mentions is a post nobody is writing; the morning pick should weigh it"' } },
  { runId: 'run-learn', stepSeq: 4, atMillis: T0 + 9 * 60_000, step: { kind: 'tool_call', status: 'ok', label: 'files.write', detail: 'workspace/Goals.md', result: 'goal 2 reworded to match the goals ledger' } },
  { runId: 'run-learn', stepSeq: 5, atMillis: T0 + 10 * 60_000, step: { kind: 'note', status: 'ok', label: 'change_needed = true: the scout prompt says nothing about releases' } },
  { runId: 'run-revise', stepSeq: 1, atMillis: T0 + 11 * 60_000 + 8_000, step: { kind: 'thinking', status: 'ok', label: 'The smallest change the learning justifies is one line in prompts/scout.md' } },
  { runId: 'run-revise', stepSeq: 2, atMillis: T0 + 13 * 60_000, step: { kind: 'tool_call', status: 'ok', label: 'files.edit', detail: 'company/teeny/agents/prompts/scout.md', result: '+2 lines' } },
  { runId: 'run-revise', stepSeq: 3, atMillis: T0 + 15 * 60_000, step: { kind: 'tool_call', status: 'ok', label: 'shell', detail: 'opencompany check /data/company/teeny', result: 'ok' } },
  { runId: 'run-revise', stepSeq: 4, atMillis: T0 + 16 * 60_000, step: { kind: 'tool_call', status: 'ok', label: 'ledger.record', detail: 'changes: "scout: watch for releases from the repos we cheer on"' } },
  { runId: 'run-revise', stepSeq: 5, atMillis: T0 + 18 * 60_000, step: { kind: 'tool_call', status: 'ok', label: 'shell', detail: 'touch /data/.reload-requested', result: 'reload requested' } },
]

const SAMPLE: BrainSnapshot = {
  source: 'sample',
  generatedAt: '2026-09-14T07:00:00Z',
  company: { name: 'Teeny', output: 'A self-revising mind: goals, tasks, skills and roster kept current by the agents that use them', slug: 'teeny' },
  agents: SAMPLE_AGENTS,
  workflows: [SAMPLE_WORKFLOW],
  runs: SAMPLE_RUNS,
  steps: SAMPLE_STEPS,
}

/**
 * The live snapshot is GET /brain on the bot. A bot that has a brain mounted
 * but no record yet (the host has not run) answers with the definition and
 * empty runs; the page keeps that as live and animates the workflow's own
 * shape instead of a run.
 */
export async function loadBrain(): Promise<BrainSnapshot> {
  const snapshot = await readApi<BrainSnapshot>('/brain', {
    check: (body) => Array.isArray((body as BrainSnapshot).agents) && Array.isArray((body as BrainSnapshot).workflows),
  })
  if (!snapshot || snapshot.agents.length === 0) return SAMPLE
  return { ...snapshot, source: 'live' }
}

export interface RepoCommit {
  sha: string
  message: string
  author: string
  at: string
  url: string
}

export interface RepoInfo {
  name: string
  url: string
  description?: string
  stars: number
  pushedAt?: string
  defaultBranch: string
  commits: RepoCommit[]
  /** True when GitHub answered; false when the numbers are placeholders. */
  live: boolean
}

const REPO_FALLBACK: RepoInfo = {
  name: BRAIN_REPO,
  url: BRAIN_REPO_URL,
  description: 'My brain, built entirely on OpenCompany',
  stars: 0,
  defaultBranch: 'main',
  commits: [],
  live: false,
}

/**
 * The repository as GitHub describes it, and its last few commits. Public
 * API, unauthenticated, revalidated every ten minutes: sixty calls an hour
 * is plenty for two calls per rebuild. `GITHUB_TOKEN`, if set, raises that.
 */
export async function loadRepo(): Promise<RepoInfo> {
  const headers: Record<string, string> = { accept: 'application/vnd.github+json' }
  if (process.env.GITHUB_TOKEN) headers.authorization = `Bearer ${process.env.GITHUB_TOKEN}`
  try {
    const [repoRes, commitsRes] = await Promise.all([
      fetch(`https://api.github.com/repos/${BRAIN_REPO}`, { headers, next: { revalidate: 600 }, signal: AbortSignal.timeout(6_000) }),
      fetch(`https://api.github.com/repos/${BRAIN_REPO}/commits?per_page=8`, { headers, next: { revalidate: 600 }, signal: AbortSignal.timeout(6_000) }),
    ])
    if (!repoRes.ok) return REPO_FALLBACK
    const repo = (await repoRes.json()) as {
      full_name: string
      html_url: string
      description?: string
      stargazers_count: number
      pushed_at?: string
      default_branch: string
    }
    const commits = commitsRes.ok
      ? ((await commitsRes.json()) as Array<{
          sha: string
          html_url: string
          commit: { message: string; author?: { name?: string; date?: string } }
          author?: { login?: string }
        }>)
      : []
    return {
      name: repo.full_name,
      url: repo.html_url,
      ...(repo.description ? { description: repo.description } : {}),
      stars: repo.stargazers_count,
      ...(repo.pushed_at ? { pushedAt: repo.pushed_at } : {}),
      defaultBranch: repo.default_branch,
      commits: commits.map((c) => ({
        sha: c.sha,
        message: c.commit.message.split('\n')[0] ?? '',
        author: c.author?.login ?? c.commit.author?.name ?? 'unknown',
        at: c.commit.author?.date ?? '',
        url: c.html_url,
      })),
      live: true,
    }
  } catch {
    return REPO_FALLBACK
  }
}
