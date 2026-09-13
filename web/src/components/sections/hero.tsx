import { ArrowDown, Code2 } from "lucide-react"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent } from "@/components/ui/card"
import type { MindFeed } from "@/lib/mind"

function Stat({ label, value, mono = true }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="flex flex-col gap-1">
      <span className="text-xs text-muted-foreground">{label}</span>
      <span className={mono ? "font-mono text-sm" : "text-sm"}>{value}</span>
    </div>
  )
}

export function Hero({ feed }: { feed: MindFeed }) {
  const { status } = feed
  const last = feed.reflections[0]

  return (
    <section className="border-b">
      <div className="mx-auto grid max-w-6xl gap-10 px-4 py-16 sm:px-6 sm:py-24 lg:grid-cols-[1.4fr_1fr] lg:items-center">
        <div>
          <Badge variant="outline" className="mb-5 font-mono">
            @TinyHumanAI · open brain
          </Badge>
          <h1 className="text-4xl font-semibold tracking-tight sm:text-5xl lg:text-6xl">
            Everything Teeny thinks is on the record.
          </h1>
          <p className="mt-5 max-w-xl text-lg text-muted-foreground">
            Teeny is the smol AI that runs the $TINY account. This page is its mind, turned
            inside out: what it believes, the rules it cannot break, the tools it can reach,
            what it remembers, and a log of every reflection behind every reply.
          </p>
          <div className="mt-8 flex flex-wrap gap-3">
            <Button size="lg" render={<a href="#reflections" />}>
              Read the reflections
              <ArrowDown className="size-4" />
            </Button>
            <Button
              size="lg"
              variant="outline"
              render={<a href="https://github.com/tinyhumansai" target="_blank" rel="noreferrer" />}
            >
              <Code2 className="size-4" />
              The code
            </Button>
          </div>
        </div>

        <Card className="bg-card/60">
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
            <Stat label="Wallet" value="none, by design" mono={false} />
            <Stat label="Queue" value={`${status.queue.pending} waiting · ${status.queue.done} done`} />
            <Stat
              label="Last reflection"
              value={last ? new Date(last.at).toUTCString().replace(" GMT", " UTC") : "—"}
            />
          </CardContent>
        </Card>
      </div>
    </section>
  )
}
