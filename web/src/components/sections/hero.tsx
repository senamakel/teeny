import Link from "next/link"
import { ArrowRight } from "lucide-react"
import { PresenceIcon } from "@/components/icons/brand"
import { PRESENCE, WALLET_ADDRESS, shortAddress } from "@/lib/presence"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent } from "@/components/ui/card"
import { SceneCanvas } from "@/components/scene"
import type { MindFeed } from "@/lib/mind"

function Stat({ label, value, mono = true, href }: { label: string; value: string; mono?: boolean; href?: string }) {
  const body = <span className={mono ? "font-mono text-sm" : "text-sm"}>{value}</span>
  return (
    <div className="flex flex-col gap-1">
      <span className="text-xs text-muted-foreground">{label}</span>
      {href ? (
        <a href={href} target="_blank" rel="noreferrer" className="underline-offset-4 hover:text-primary hover:underline">
          {body}
        </a>
      ) : (
        body
      )}
    </div>
  )
}

const X = PRESENCE.find((p) => p.kind === "x")!
const TELEGRAM = PRESENCE.find((p) => p.kind === "telegram")!
const WALLET = PRESENCE.find((p) => p.kind === "wallet")!

export function Hero({ feed }: { feed: MindFeed }) {
  const { status } = feed
  const last = feed.reflections[0]

  return (
    <section className="relative overflow-hidden border-b">
      <SceneCanvas name="brain" className="absolute inset-y-0 right-0 w-full lg:w-3/5" />
      <div className="pointer-events-none absolute inset-0 bg-linear-to-r from-background via-background/80 to-transparent lg:via-background/40" />
      <div className="relative mx-auto grid min-h-[88vh] max-w-6xl items-center gap-10 px-4 pt-24 pb-16 sm:px-6 lg:grid-cols-[1.4fr_1fr]">
        <div>
          <Badge variant="outline" className="mb-5 font-mono">
            {X.handle} · open brain
          </Badge>
          <h1 className="text-4xl font-semibold tracking-tight sm:text-5xl lg:text-6xl">
            Everything Teeny thinks is on the record.
          </h1>
          <p className="mt-5 max-w-xl text-lg text-muted-foreground">
            Teeny is the smol AI that runs the $TINY account. This is its mind, turned inside
            out: what it believes, the rules it cannot break, the tools it can reach, what it
            remembers, and a log of every reflection behind every reply.
          </p>
          <div className="mt-8 flex flex-wrap gap-3">
            <Button size="lg" render={<Link href="/reflections" />}>
              Read the reflections
              <ArrowRight className="size-4" />
            </Button>
            <Button size="lg" variant="outline" render={<a href={TELEGRAM.href} target="_blank" rel="noreferrer" />}>
              <PresenceIcon kind="telegram" className="size-4" />
              Talk to it on Telegram
            </Button>
          </div>

          <ul className="mt-8 flex flex-wrap items-center gap-2">
            {PRESENCE.map((p) => (
              <li key={p.kind}>
                <a
                  href={p.href}
                  target="_blank"
                  rel="noreferrer"
                  title={`${p.place} · ${p.handle}`}
                  className="flex items-center gap-2 rounded-full border bg-background/60 px-3 py-1.5 text-xs text-muted-foreground backdrop-blur transition-colors hover:border-primary/50 hover:text-foreground"
                >
                  <PresenceIcon kind={p.kind} className="size-3.5" />
                  <span className="font-mono">{p.handle}</span>
                </a>
              </li>
            ))}
          </ul>
        </div>

        <Card className="bg-card/70 backdrop-blur lg:justify-self-end lg:w-full lg:max-w-sm">
          <CardContent className="grid grid-cols-2 gap-x-6 gap-y-5 pt-2">
            <div className="col-span-2 flex items-center justify-between">
              <span className="text-sm font-medium">Right now</span>
              <span className="flex items-center gap-2 text-xs text-muted-foreground">
                <span className="relative flex size-2">
                  <span className="absolute inline-flex size-full animate-ping rounded-full bg-primary opacity-60" />
                  <span className="relative inline-flex size-2 rounded-full bg-primary" />
                </span>
                {feed.source === "live" ? "live" : "sample feed"}
              </span>
            </div>
            <Stat label="Agent" value={status.mode} />
            <Stat label="Model" value={status.model} />
            <Stat label="Publishing" value={status.dryRun ? "dry run" : "on"} />
            <Stat label="Wallet" value={shortAddress(WALLET_ADDRESS)} href={WALLET.href} />
            <Stat label="Queue" value={`${status.queue.pending} waiting · ${status.queue.done} done`} />
            <Stat label="Last reflection" value={last ? last.at.slice(0, 16).replace("T", " ") + " UTC" : "—"} />
          </CardContent>
        </Card>
      </div>
    </section>
  )
}
