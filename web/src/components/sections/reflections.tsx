import { AlertCircle, BarChart3, MessageCircle, Terminal } from "lucide-react"
import { Badge } from "@/components/ui/badge"
import { Card, CardContent, CardHeader } from "@/components/ui/card"
import { Separator } from "@/components/ui/separator"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { Container } from "@/components/section"
import type { MindFeed, Outcome, Reflection, Tone, Trigger } from "@/lib/mind"

const TRIGGER_ICON: Record<Trigger, typeof MessageCircle> = {
  mention: MessageCircle,
  chart: BarChart3,
  task: Terminal,
}

const TONE_STYLE: Record<Tone, string> = {
  friendly: "border-primary/40 text-primary",
  neutral: "",
  confused: "border-sky-400/40 text-sky-400",
  negative: "border-orange-400/40 text-orange-400",
  hostile: "border-destructive/40 text-destructive",
}

const OUTCOME_LABEL: Record<Outcome, string> = {
  posted: "posted",
  "dry-run": "dry run",
  skipped: "no reply",
  held: "held",
}

function when(iso: string): string {
  const d = new Date(iso)
  return d.toISOString().slice(0, 16).replace("T", " ") + " UTC"
}

function Label({ children }: { children: React.ReactNode }) {
  return (
    <p className="mb-1.5 font-mono text-[11px] uppercase tracking-widest text-muted-foreground">
      {children}
    </p>
  )
}

function ReflectionCard({ entry }: { entry: Reflection }) {
  const Icon = TRIGGER_ICON[entry.trigger]
  return (
    <Card className="gap-4">
      <CardHeader className="flex flex-wrap items-center gap-2">
        <span className="flex items-center gap-2 text-sm">
          <Icon className="size-4 text-muted-foreground" />
          <span className="font-medium capitalize">{entry.trigger}</span>
        </span>
        <span className="font-mono text-xs text-muted-foreground">{when(entry.at)}</span>
        <span className="ml-auto flex items-center gap-2">
          <Badge variant="outline" className={TONE_STYLE[entry.tone]}>
            {entry.tone}
          </Badge>
          <Badge variant={entry.outcome === "posted" ? "default" : "secondary"}>
            {OUTCOME_LABEL[entry.outcome]}
          </Badge>
        </span>
      </CardHeader>

      <CardContent className="grid gap-5 lg:grid-cols-[1fr_1.3fr]">
        <div className="flex flex-col gap-5">
          <div>
            <Label>Incoming</Label>
            <p className="rounded-md bg-muted/60 px-3 py-2 text-sm">{entry.incoming}</p>
          </div>

          <div>
            <Label>Recalled</Label>
            {entry.recalled.length ? (
              <ul className="flex flex-col gap-1 text-sm text-muted-foreground">
                {entry.recalled.map((line) => (
                  <li key={line} className="flex gap-2">
                    <span className="text-primary">·</span>
                    {line}
                  </li>
                ))}
              </ul>
            ) : (
              <p className="text-sm text-muted-foreground">nothing, on purpose</p>
            )}
          </div>

          <div>
            <Label>Tools</Label>
            {entry.tools.length ? (
              <ul className="flex flex-col gap-1.5">
                {entry.tools.map((call) => (
                  <li key={call.name} className="flex flex-wrap items-baseline gap-x-2 text-sm">
                    <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-xs">{call.name}</code>
                    <span className="text-muted-foreground">{call.result}</span>
                  </li>
                ))}
              </ul>
            ) : (
              <p className="text-sm text-muted-foreground">none</p>
            )}
          </div>
        </div>

        <div className="flex flex-col gap-5">
          <div>
            <Label>Thinking</Label>
            <p className="text-sm leading-relaxed">{entry.thinking}</p>
          </div>

          <div>
            <Label>{entry.trigger === "chart" ? "Posted" : "Said"}</Label>
            {entry.said ? (
              <blockquote className="border-l-2 border-primary pl-3 text-sm italic">
                {entry.said}
              </blockquote>
            ) : (
              <p className="text-sm text-muted-foreground">
                nothing{entry.outcomeNote ? ` — ${entry.outcomeNote}` : ""}
              </p>
            )}
            {entry.said && entry.outcomeNote ? (
              <p className="mt-2 text-xs text-muted-foreground">{entry.outcomeNote}</p>
            ) : null}
          </div>

          {entry.declined.length ? (
            <div>
              <Label>Decided against</Label>
              <ul className="flex flex-col gap-1 text-sm text-muted-foreground">
                {entry.declined.map((line) => (
                  <li key={line} className="flex gap-2">
                    <span className="text-destructive">×</span>
                    {line}
                  </li>
                ))}
              </ul>
            </div>
          ) : null}
        </div>
      </CardContent>
    </Card>
  )
}

function Feed({ entries }: { entries: Reflection[] }) {
  if (!entries.length) {
    return (
      <Card>
        <CardContent className="py-10 text-center text-sm text-muted-foreground">
          Nothing here yet.
        </CardContent>
      </Card>
    )
  }
  return (
    <div className="flex flex-col gap-4">
      {entries.map((entry) => (
        <ReflectionCard key={entry.id} entry={entry} />
      ))}
    </div>
  )
}

export function Reflections({ feed }: { feed: MindFeed }) {
  const all = feed.reflections
  const replies = all.filter((e) => e.trigger === "mention" && e.outcome !== "skipped" && e.outcome !== "held")
  const charts = all.filter((e) => e.trigger === "chart")
  const quiet = all.filter((e) => e.outcome === "skipped" || e.outcome === "held")

  return (
    <Container>
      {feed.source === "sample" ? (
        <div className="mb-6 flex items-start gap-3 rounded-lg border border-primary/30 bg-primary/5 px-4 py-3 text-sm">
          <AlertCircle className="mt-0.5 size-4 shrink-0 text-primary" />
          <p>
            <span className="font-medium">Sample entries.</span>{" "}
            <span className="text-muted-foreground">
              The live feed is not connected yet. These are reconstructed from incidents
              recorded in the code and its history, so the shape is real even if the
              timestamps are not.
            </span>
          </p>
        </div>
      ) : null}

      <Tabs defaultValue="all">
        <TabsList variant="line" className="mb-6">
          <TabsTrigger value="all">All · {all.length}</TabsTrigger>
          <TabsTrigger value="replies">Replies · {replies.length}</TabsTrigger>
          <TabsTrigger value="charts">Chart reads · {charts.length}</TabsTrigger>
          <TabsTrigger value="quiet">Stayed quiet · {quiet.length}</TabsTrigger>
        </TabsList>
        <TabsContent value="all"><Feed entries={all} /></TabsContent>
        <TabsContent value="replies"><Feed entries={replies} /></TabsContent>
        <TabsContent value="charts"><Feed entries={charts} /></TabsContent>
        <TabsContent value="quiet"><Feed entries={quiet} /></TabsContent>
      </Tabs>

      <Separator className="my-8" />
      <p className="max-w-2xl text-sm text-muted-foreground">
        The three things that never appear in this log: a price prediction, another token,
        or a wallet action. The first two are rules. The third is not possible: there is no
        tool that could do it.
      </p>
    </Container>
  )
}
