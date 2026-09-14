'use client'

import { useEffect, useMemo, useRef, useState } from "react"
import { Bot, Clock, GitBranch, Send, Wrench, Zap } from "lucide-react"
import type { BrainAgent, BrainRun, BrainSnapshot, BrainStep, BrainWorkflow } from "@/lib/company"
import { compactTokens } from "@/lib/ledger"
import { useReducedMotion } from "@/lib/motion"
import { cn } from "@/lib/utils"

/**
 * The desk: the brain's agents at work, as a little window you can watch.
 *
 * The graph is the workflow as defined in the brain's checkout, laid out by
 * rank. The play is the workflow's latest run, replayed from the host's own
 * step record: a packet travels the edge into each agent as its run starts,
 * the desk lights up, and its steps print on the right, one at a time, at a
 * pace a person can read. With no run recorded yet it rehearses the
 * workflow from its own node summaries, and says so, so the shape of what
 * will happen is visible before it has happened once.
 */

interface Placed {
  id: string
  kind: string
  name: string
  agent?: string
  summary?: string
  schedule?: string
  x: number
  y: number
}

const NODE_W = 124
const NODE_H = 64
/** Room for one column and one row; the canvas grows with the graph. */
const COL = 156
const ROW = 96

interface Graph {
  width: number
  height: number
  nodes: Placed[]
  edges: Array<{ from: Placed; to: Placed; label?: string; d: string }>
}

/** Columns by longest path from a source; rows spread evenly within a column. */
function layout(workflow: BrainWorkflow): Graph {
  const rank = new Map<string, number>()
  const incoming = new Map<string, string[]>()
  for (const node of workflow.nodes) incoming.set(node.id, [])
  for (const edge of workflow.edges) incoming.get(edge.to)?.push(edge.from)
  const visit = (id: string, seen: Set<string>): number => {
    const known = rank.get(id)
    if (known !== undefined) return known
    if (seen.has(id)) return 0
    seen.add(id)
    const parents = incoming.get(id) ?? []
    const r = parents.length ? Math.max(...parents.map((p) => visit(p, seen))) + 1 : 0
    rank.set(id, r)
    return r
  }
  for (const node of workflow.nodes) visit(node.id, new Set())
  const columns = Math.max(...workflow.nodes.map((n) => rank.get(n.id) ?? 0)) + 1
  const byColumn = new Map<number, string[]>()
  for (const node of workflow.nodes) {
    const r = rank.get(node.id) ?? 0
    byColumn.set(r, [...(byColumn.get(r) ?? []), node.id])
  }
  const rows = Math.max(...[...byColumn.values()].map((ids) => ids.length), 1)
  const width = Math.max(columns * COL + 70, 640)
  const height = rows * ROW + 60
  const gapX = (width - 40) / Math.max(columns, 1)
  const nodes: Placed[] = workflow.nodes.map((node) => {
    const r = rank.get(node.id) ?? 0
    const siblings = byColumn.get(r) ?? [node.id]
    const i = siblings.indexOf(node.id)
    const slot = (height - 40) / (siblings.length + 1)
    return { ...node, x: 20 + gapX * r + gapX / 2, y: 20 + slot * (i + 1) }
  })
  const at = new Map(nodes.map((n) => [n.id, n]))
  const edges = workflow.edges
    .map((edge) => {
      const from = at.get(edge.from)
      const to = at.get(edge.to)
      if (!from || !to) return undefined
      const x1 = from.x + NODE_W / 2 - 6
      const x2 = to.x - NODE_W / 2 + 6
      const c = (x2 - x1) / 2
      return { from, to, ...(edge.label ? { label: edge.label } : {}), d: `M ${x1} ${from.y} C ${x1 + c} ${from.y}, ${x2 - c} ${to.y}, ${x2} ${to.y}` }
    })
    .filter((e): e is NonNullable<typeof e> => Boolean(e))
  return { width, height, nodes, edges }
}

type Event =
  | { kind: "handoff"; from: string; to: string; ms: number; at?: number }
  | { kind: "step"; node: string; agent: string; step: BrainStep["step"]; ms: number; at?: number; tokens?: number }
  | { kind: "rest"; ms: number }

/** The latest workflow run's steps, in order, with a handoff before each run. */
function fromRecord(workflow: BrainWorkflow, runs: BrainRun[], steps: BrainStep[]): Event[] | undefined {
  const inWorkflow = runs.filter((r) => r.workflowRunId && r.nodeId && workflow.nodes.some((n) => n.id === r.nodeId))
  if (!inWorkflow.length) return undefined
  const latest = inWorkflow.reduce((a, b) => (b.createdAtMillis > a.createdAtMillis ? b : a)).workflowRunId
  const ordered = inWorkflow.filter((r) => r.workflowRunId === latest).sort((a, b) => a.createdAtMillis - b.createdAtMillis)
  const nodeAt = new Map(workflow.nodes.map((n) => [n.id, n]))
  const events: Event[] = []
  const start = workflow.nodes.find((n) => n.kind === "trigger")?.id ?? ordered[0]?.nodeId ?? ""
  let previous = start
  for (const run of ordered) {
    const node = run.nodeId!
    events.push({ kind: "handoff", from: previous, to: node, ms: 1_300, at: run.createdAtMillis })
    const own = steps.filter((s) => s.runId === run.id).sort((a, b) => a.stepSeq - b.stepSeq)
    const tokens = (run.usage?.input ?? 0) + (run.usage?.output ?? 0)
    if (own.length) {
      own.forEach((s, i) => {
        events.push({ kind: "step", node, agent: run.agentId, step: s.step, ms: 1_700, at: s.atMillis, ...(i === own.length - 1 && tokens ? { tokens } : {}) })
      })
    } else {
      const summary = nodeAt.get(node)?.summary ?? nodeAt.get(node)?.name ?? node
      events.push({ kind: "step", node, agent: run.agentId, step: { kind: "note", status: run.status, label: summary }, ms: 2_000, ...(tokens ? { tokens } : {}) })
    }
    previous = node
  }
  const outputs = workflow.nodes.filter((n) => n.kind === "output")
  const last = ordered[ordered.length - 1]
  if (outputs[0] && last?.nodeId) events.push({ kind: "handoff", from: last.nodeId, to: outputs[0].id, ms: 1_300 })
  events.push({ kind: "rest", ms: 3_500 })
  return events
}

/** The workflow rehearsed from its own definition: one pass down the "yes" branches. */
function rehearsal(workflow: BrainWorkflow): Event[] {
  const events: Event[] = []
  const next = new Map<string, Array<{ to: string; label?: string }>>()
  for (const edge of workflow.edges) next.set(edge.from, [...(next.get(edge.from) ?? []), { to: edge.to, ...(edge.label ? { label: edge.label } : {}) }])
  let current = workflow.nodes.find((n) => n.kind === "trigger") ?? workflow.nodes[0]
  const seen = new Set<string>()
  while (current && !seen.has(current.id)) {
    seen.add(current.id)
    if (current.kind === "agent" && current.agent) {
      events.push({ kind: "step", node: current.id, agent: current.agent, step: { kind: "thinking", status: "ok", label: current.summary ?? current.name }, ms: 2_400 })
    }
    const options = next.get(current.id) ?? []
    const chosen = options.find((o) => o.label !== "no") ?? options[0]
    if (!chosen) break
    const to = workflow.nodes.find((n) => n.id === chosen.to)
    if (!to) break
    events.push({ kind: "handoff", from: current.id, to: to.id, ms: 1_300 })
    current = to
  }
  events.push({ kind: "rest", ms: 3_500 })
  return events
}

const STEP_ICON: Record<string, typeof Zap> = { tool_call: Wrench, thinking: Zap, note: Bot }

function clock(ms: number | undefined): string {
  if (!ms) return "--:--:--"
  return new Date(ms).toISOString().slice(11, 19)
}

export function AgentDesk({ snapshot, className }: { snapshot: BrainSnapshot; className?: string }) {
  const workflow = snapshot.workflows[0]
  const agents = useMemo(() => new Map(snapshot.agents.map((a) => [a.id, a])), [snapshot.agents])
  const graph = useMemo<Graph>(() => (workflow ? layout(workflow) : { width: 640, height: 200, nodes: [], edges: [] }), [workflow])
  const recorded = useMemo(() => (workflow ? fromRecord(workflow, snapshot.runs, snapshot.steps) : undefined), [workflow, snapshot.runs, snapshot.steps])
  const events = useMemo(() => recorded ?? (workflow ? rehearsal(workflow) : []), [recorded, workflow])
  const [cursor, setCursor] = useState(0)
  const still = useReducedMotion()
  const logRef = useRef<HTMLOListElement>(null)

  useEffect(() => {
    if (still || !events.length) return
    const current = events[cursor % events.length]
    const timer = setTimeout(() => setCursor((c) => (c + 1) % events.length), current?.ms ?? 1_500)
    return () => clearTimeout(timer)
  }, [cursor, events, still])

  useEffect(() => {
    logRef.current?.scrollTo({ top: logRef.current.scrollHeight, behavior: "smooth" })
  }, [cursor])

  if (!workflow) return null

  const index = still ? events.length : cursor
  const played = events.slice(0, Math.min(index + 1, events.length))
  const current = still ? undefined : events[index]
  const lit = new Set<string>()
  if (current?.kind === "step") lit.add(current.node)
  if (current?.kind === "handoff") lit.add(current.to)
  const done = new Set<string>()
  for (const e of played) if (e.kind === "step") done.add(e.node)
  const log = played.filter((e): e is Extract<Event, { kind: "step" }> => e.kind === "step").slice(-40)
  const tokensByNode = new Map<string, number>()
  for (const e of played) if (e.kind === "step" && e.tokens) tokensByNode.set(e.node, e.tokens)
  const runId = snapshot.runs.find((r) => r.workflowRunId && r.nodeId)?.workflowRunId

  return (
    <div className={cn("overflow-hidden rounded-2xl border bg-card/80 shadow-2xl shadow-black/30 backdrop-blur", className)}>
      {/* title bar */}
      <div className="flex items-center gap-3 border-b bg-muted/40 px-4 py-2.5">
        <span className="flex gap-1.5">
          <span className="size-2.5 rounded-full bg-destructive/70" />
          <span className="size-2.5 rounded-full bg-primary/70" />
          <span className="size-2.5 rounded-full bg-emerald-400/70" />
        </span>
        <span className="font-mono text-xs text-muted-foreground">
          {snapshot.company.name.toLowerCase()} brain · {workflow.name.toLowerCase()}
          {runId ? ` · ${runId}` : ""}
        </span>
        <span className="ml-auto flex items-center gap-2 font-mono text-[11px] uppercase tracking-widest text-muted-foreground">
          <span className={cn("size-1.5 rounded-full", recorded ? "bg-emerald-400" : "bg-primary")} />
          {recorded ? "replaying the latest run" : "rehearsal · no run recorded yet"}
        </span>
      </div>

      <div className="grid lg:grid-cols-[1.9fr_1fr]">
        {/* the floor */}
        <div className="overflow-x-auto">
        <svg viewBox={`0 0 ${graph.width} ${graph.height}`} className="w-full min-w-[560px] self-center" role="img" aria-label={`The ${workflow.name} workflow: ${workflow.nodes.map((n) => n.name).join(", ")}`}>
          <defs>
            <marker id="desk-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse">
              <path d="M 0 0 L 10 5 L 0 10 z" className="fill-muted-foreground/50" />
            </marker>
            <radialGradient id="desk-glow">
              <stop offset="0%" stopColor="currentColor" stopOpacity="0.35" />
              <stop offset="100%" stopColor="currentColor" stopOpacity="0" />
            </radialGradient>
          </defs>
          {graph.edges.map((edge, i) => {
            const active = current?.kind === "handoff" && current.from === edge.from.id && current.to === edge.to.id
            return (
              <g key={i}>
                <path d={edge.d} fill="none" className={cn("stroke-border transition-colors", active && "stroke-primary/70")} strokeWidth={active ? 2 : 1.5} markerEnd="url(#desk-arrow)" />
                {edge.label ? (
                  <text x={(edge.from.x + edge.to.x) / 2} y={(edge.from.y + edge.to.y) / 2 - 6} textAnchor="middle" className="fill-muted-foreground font-mono text-[10px]">
                    {edge.label}
                  </text>
                ) : null}
                {active && !still ? (
                  <circle key={`packet-${index}`} r={5} className="fill-primary">
                    <animateMotion dur={`${current.ms}ms`} repeatCount="1" fill="freeze" path={edge.d} />
                  </circle>
                ) : null}
              </g>
            )
          })}
          {graph.nodes.map((node) => (
            <Desk key={node.id} node={node} agent={node.agent ? agents.get(node.agent) : undefined} lit={lit.has(node.id)} done={done.has(node.id)} tokens={tokensByNode.get(node.id)} />
          ))}
        </svg>
        </div>

        {/* the log */}
        <div className="flex min-h-56 flex-col border-t bg-background/60 lg:border-t-0 lg:border-l">
          <p className="border-b px-4 py-2 font-mono text-[11px] uppercase tracking-widest text-muted-foreground">run log</p>
          <ol ref={logRef} className="flex max-h-72 flex-1 flex-col gap-2 overflow-y-auto px-4 py-3 font-mono text-[11px] leading-relaxed">
            {log.length ? (
              log.map((e, i) => {
                const Icon = STEP_ICON[e.step.kind] ?? Bot
                const last = i === log.length - 1 && !still
                return (
                  <li key={`${e.node}-${i}`} className={cn("flex gap-2", last ? "text-foreground" : "text-muted-foreground")}>
                    <span className="shrink-0 text-muted-foreground/70">{clock(e.at)}</span>
                    <span className="shrink-0 text-primary">{e.agent}</span>
                    <Icon className="mt-0.5 size-3 shrink-0" />
                    <span className="min-w-0 break-words">
                      {e.step.label}
                      {e.step.detail ? <span className="text-muted-foreground"> · {e.step.detail}</span> : null}
                      {e.step.result ? <span className="text-emerald-400/90"> → {e.step.result}</span> : null}
                      {e.tokens ? <span className="text-sky-400"> · {compactTokens(e.tokens)} tok</span> : null}
                    </span>
                  </li>
                )
              })
            ) : (
              <li className="text-muted-foreground">waiting for the first packet…</li>
            )}
            {!still && current?.kind === "handoff" ? (
              <li className="flex gap-2 text-muted-foreground">
                <span className="shrink-0 text-muted-foreground/70">{clock(current.at)}</span>
                <Send className="mt-0.5 size-3 shrink-0" />
                <span>{current.from} → {current.to}</span>
              </li>
            ) : null}
          </ol>
        </div>
      </div>
    </div>
  )
}

function Desk({ node, agent, lit, done, tokens }: { node: Placed; agent: BrainAgent | undefined; lit: boolean; done: boolean; tokens: number | undefined }) {
  const x = node.x - NODE_W / 2
  const y = node.y - NODE_H / 2
  if (node.kind === "trigger") {
    return (
      <g transform={`translate(${node.x}, ${node.y})`} className={cn(lit ? "text-primary" : "text-muted-foreground")}>
        <circle r={22} className={cn("fill-card stroke-border", lit && "stroke-primary")} strokeWidth={1.5} />
        <Clock x={-8} y={-8} width={16} height={16} className="stroke-current" />
        <text y={38} textAnchor="middle" className="fill-muted-foreground font-mono text-[10px]">{node.schedule ?? node.name}</text>
      </g>
    )
  }
  if (node.kind === "condition") {
    return (
      <g transform={`translate(${node.x}, ${node.y})`} className={cn(lit ? "text-primary" : "text-muted-foreground")}>
        <rect x={-18} y={-18} width={36} height={36} rx={6} transform="rotate(45)" className={cn("fill-card stroke-border", lit && "stroke-primary")} strokeWidth={1.5} />
        <GitBranch x={-8} y={-8} width={16} height={16} className="stroke-current" />
        <text y={40} textAnchor="middle" className="fill-muted-foreground font-mono text-[10px]">{node.name.length > 26 ? "decide" : node.name}</text>
      </g>
    )
  }
  if (node.kind === "output") {
    return (
      <g transform={`translate(${node.x}, ${node.y})`} className={cn(lit ? "text-primary" : "text-muted-foreground")}>
        <rect x={-40} y={-20} width={80} height={40} rx={10} className={cn("fill-card stroke-border", lit && "stroke-primary")} strokeWidth={1.5} />
        <Send x={-28} y={-8} width={16} height={16} className="stroke-current" />
        <text x={-6} y={4} className="fill-foreground font-mono text-[10px]">operator</text>
      </g>
    )
  }
  return (
    <g className={cn(lit ? "text-primary" : "text-muted-foreground")}>
      {lit ? <circle cx={node.x} cy={node.y} r={70} fill="url(#desk-glow)" /> : null}
      <rect x={x} y={y} width={NODE_W} height={NODE_H} rx={12} className={cn("fill-card stroke-border transition-all", lit && "stroke-primary", done && !lit && "stroke-emerald-400/50")} strokeWidth={1.5} />
      <circle cx={x + 22} cy={node.y} r={13} className={cn("fill-muted", lit && "fill-primary/20")} />
      <Bot x={x + 14} y={node.y - 8} width={16} height={16} className="stroke-current" />
      <text x={x + 42} y={node.y - 4} className="fill-foreground text-[12px] font-semibold">{agent?.role ?? node.agent ?? node.id}</text>
      <text x={x + 42} y={node.y + 11} className="fill-muted-foreground font-mono text-[9px]">
        {tokens ? `${compactTokens(tokens)} tok` : lit ? "working…" : done ? "done" : "idle"}
      </text>
      {lit ? (
        <circle cx={x + NODE_W - 12} cy={y + 12} r={3} className="fill-primary">
          <animate attributeName="opacity" values="1;0.2;1" dur="1.2s" repeatCount="indefinite" />
        </circle>
      ) : null}
    </g>
  )
}
