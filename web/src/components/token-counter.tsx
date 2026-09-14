'use client'

import { useEffect, useRef, useState } from "react"
import { LANES, LANE_META, compactTokens, formatTokens, type Lane, type Totals } from "@/lib/ledger"
import { useReducedMotion } from "@/lib/motion"
import { cn } from "@/lib/utils"

/**
 * The counter: every token both minds have ever burned, one number per mind
 * and one for the pair. It polls the bot's totals through /api/pulse and
 * rolls each figure from where it was to where it is now, so a row landing
 * in the ledger shows up here within half a minute as the number moving.
 * The figures are never extrapolated: between polls they hold still.
 */

const POLL_MS = 20_000
const ROLL_MS = 1_400

function useRolling(target: number): number {
  const reduced = useReducedMotion()
  const [shown, setShown] = useState(target)
  const from = useRef(target)
  const start = useRef<number | null>(null)
  useEffect(() => {
    if (reduced) return
    const origin = from.current
    if (origin === target) return
    start.current = null
    let frame = 0
    const step = (now: number) => {
      if (start.current === null) start.current = now
      const t = Math.min((now - start.current) / ROLL_MS, 1)
      const eased = 1 - Math.pow(1 - t, 3)
      const value = Math.round(origin + (target - origin) * eased)
      setShown(value)
      if (t < 1) frame = requestAnimationFrame(step)
      else from.current = target
    }
    frame = requestAnimationFrame(step)
    return () => cancelAnimationFrame(frame)
  }, [target, reduced])
  return reduced ? target : shown
}

function Digits({ value, className }: { value: number; className?: string }) {
  const shown = useRolling(value)
  return (
    <span className={cn("font-mono tabular-nums tracking-tight", className)} aria-live="off">
      {formatTokens(shown)}
    </span>
  )
}

function ago(iso: string | undefined, now: number): string {
  if (!iso) return "no call yet"
  const ms = now - Date.parse(iso)
  if (!Number.isFinite(ms) || ms < 0) return "just now"
  const s = Math.floor(ms / 1000)
  if (s < 60) return `${s}s ago`
  const m = Math.floor(s / 60)
  if (m < 60) return `${m}m ago`
  const h = Math.floor(m / 60)
  if (h < 48) return `${h}h ago`
  return `${Math.floor(h / 24)}d ago`
}

function LaneCard({ lane, totals, now }: { lane: Lane; totals: Totals; now: number }) {
  const meta = LANE_META[lane]
  const t = totals.lanes[lane]
  const accent = lane === "brain" ? "text-sky-400" : "text-primary"
  const border = lane === "brain" ? "hover:border-sky-400/40" : "hover:border-primary/40"
  return (
    <div className={cn("flex flex-col gap-3 rounded-2xl border bg-card/70 p-5 backdrop-blur transition-colors sm:p-6", border)}>
      <div className="flex items-baseline justify-between gap-3">
        <p className="flex items-baseline gap-2">
          <span className={cn("text-2xl font-semibold tracking-tight", accent)}>{meta.name}</span>
          <span className="text-sm text-muted-foreground">{meta.who}</span>
        </p>
        <span className="font-mono text-xs text-muted-foreground">{ago(t.last, now)}</span>
      </div>
      <Digits value={t.tokens.total} className="text-3xl sm:text-4xl lg:text-[2.6rem] lg:leading-none" />
      <p className="text-xs text-muted-foreground">tokens · {formatTokens(t.calls)} calls</p>
      <dl className="grid grid-cols-3 gap-2 border-t pt-3 font-mono text-[11px] text-muted-foreground">
        <div>
          <dt>in</dt>
          <dd className="text-foreground/80">{compactTokens(t.tokens.input)}</dd>
        </div>
        <div>
          <dt>thinking</dt>
          <dd className="text-foreground/80">{compactTokens(t.tokens.reasoning)}</dd>
        </div>
        <div>
          <dt>out</dt>
          <dd className="text-foreground/80">{compactTokens(t.tokens.output - t.tokens.reasoning)}</dd>
        </div>
      </dl>
      <p className="text-xs leading-relaxed text-muted-foreground">{meta.blurb}</p>
    </div>
  )
}

export function TokenCounter({ initial, source }: { initial: Totals; source: "live" | "sample" }) {
  const [totals, setTotals] = useState(initial)
  const [live, setLive] = useState(source === "live")
  const [now, setNow] = useState(() => Date.now())

  useEffect(() => {
    let cancelled = false
    const tick = async () => {
      try {
        const response = await fetch("/api/pulse", { cache: "no-store" })
        if (!response.ok) return
        const body = (await response.json()) as { source: "live" | "sample"; totals: Totals }
        if (cancelled) return
        setLive(body.source === "live")
        setTotals(body.totals)
        setNow(Date.now())
      } catch {
        // The last figure stands. A poll that fails is not a token unburned.
      }
    }
    const poll = setInterval(tick, POLL_MS)
    const clock = setInterval(() => setNow(Date.now()), 10_000)
    return () => {
      cancelled = true
      clearInterval(poll)
      clearInterval(clock)
    }
  }, [])

  const both = LANES.reduce((sum, lane) => sum + totals.lanes[lane].tokens.total, 0)
  const calls = LANES.reduce((sum, lane) => sum + totals.lanes[lane].calls, 0)

  return (
    <div className="flex flex-col gap-5">
      <div className="flex flex-col gap-1">
        <p className="flex items-center gap-2 font-mono text-xs uppercase tracking-widest text-muted-foreground">
          <span className="relative flex size-2">
            <span className={cn("absolute inline-flex size-full animate-ping rounded-full opacity-60", live ? "bg-primary" : "bg-muted-foreground")} />
            <span className={cn("relative inline-flex size-2 rounded-full", live ? "bg-primary" : "bg-muted-foreground")} />
          </span>
          {live ? "tokens burned, both minds, all time" : "sample count · the bot is not reachable from here"}
        </p>
        <Digits value={both} className="text-5xl font-semibold sm:text-6xl lg:text-7xl" />
        <p className="text-sm text-muted-foreground">
          across {formatTokens(calls)} model calls · every one of them is a row in the{" "}
          <a href="/ledger" className="underline underline-offset-4 hover:text-foreground">ledger</a>
        </p>
      </div>
      <div className="grid gap-4 sm:grid-cols-2">
        {LANES.map((lane) => (
          <LaneCard key={lane} lane={lane} totals={totals} now={now} />
        ))}
      </div>
    </div>
  )
}
