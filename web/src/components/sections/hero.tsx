import Link from "next/link"
import { ArrowRight } from "lucide-react"
import { PresenceIcon } from "@/components/icons/brand"
import { PRESENCE } from "@/lib/presence"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { SceneCanvas } from "@/components/scene"
import { TokenCounter } from "@/components/token-counter"
import { SAMPLE_TOTALS } from "@/lib/ledger"
import type { MindFeed } from "@/lib/mind"

const X = PRESENCE.find((p) => p.kind === "x")!
const TELEGRAM = PRESENCE.find((p) => p.kind === "telegram")!

export function Hero({ feed }: { feed: MindFeed }) {
  const { status } = feed
  return (
    <section className="relative overflow-hidden border-b">
      <SceneCanvas name="brain" className="absolute inset-y-0 right-0 w-full lg:w-3/5" />
      <div className="pointer-events-none absolute inset-0 bg-linear-to-r from-background via-background/85 to-background/40" />
      <div className="pointer-events-none absolute inset-0 bg-linear-to-t from-background via-transparent to-transparent" />
      <div className="relative mx-auto flex min-h-[92vh] max-w-6xl flex-col justify-center gap-10 px-4 pt-28 pb-16 sm:px-6">
        <div className="max-w-2xl">
          <Badge variant="outline" className="mb-5 font-mono">
            {X.handle} · open brain · {status.mode} on {status.model}
            {status.dryRun ? " · dry run" : ""}
          </Badge>
          <h1 className="text-4xl font-semibold tracking-tight sm:text-5xl lg:text-6xl">
            Everything Teeny thinks is on the record.
          </h1>
          <p className="mt-5 text-lg text-muted-foreground">
            Two minds, one account: the mascot that talks, and the brain that decides what it is trying to do.
            Every token either has ever burned is counted here, and every call behind the count is a row in
            the ledger: what went in, what it reasoned, what came out.
          </p>
        </div>

        <TokenCounter initial={feed.totals ?? SAMPLE_TOTALS} source={feed.source} />

        <div className="flex flex-wrap items-center gap-3">
          <Button size="lg" render={<Link href="/ledger" />}>
            Open the ledger
            <ArrowRight className="size-4" />
          </Button>
          <Button size="lg" variant="outline" render={<Link href="/brain" />}>
            See the brain at work
          </Button>
          <Button size="lg" variant="ghost" render={<a href={TELEGRAM.href} target="_blank" rel="noreferrer" />}>
            <PresenceIcon kind="telegram" className="size-4" />
            Talk to it
          </Button>
          <ul className="ml-auto flex flex-wrap items-center gap-2">
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
      </div>
    </section>
  )
}
