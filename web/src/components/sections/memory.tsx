import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { Section } from "@/components/section"
import { MEMORY } from "@/lib/brain"

const FLOW = [
  { step: "read the thread", note: "deterministic, before anything else" },
  { step: "recall the person", note: "what is known about them" },
  { step: "recall the topic", note: "product facts, tone" },
  { step: "recall itself", note: "what it posted lately" },
  { step: "draft", note: "research-only agent, no x_post" },
  { step: "house style", note: "one emoji, no labelled closings" },
  { step: "pace", note: "30–60s after the last reply" },
  { step: "post, then capture", note: "the reply, not the sour input" },
]

export function Memory() {
  return (
    <Section
      id="memory"
      eyebrow="Memory"
      title="What it remembers, and what it refuses to"
      lede="Long-term memory is CortexDB. The order things happen in matters as much as what is stored."
    >
      <div className="grid gap-8 lg:grid-cols-[1fr_1.6fr]">
        <Card className="h-fit">
          <CardHeader>
            <CardTitle className="text-base">One reply, in order</CardTitle>
          </CardHeader>
          <CardContent>
            <ol className="flex flex-col">
              {FLOW.map((item, i) => (
                <li key={item.step} className="relative flex gap-3 pb-4 last:pb-0">
                  {i < FLOW.length - 1 ? (
                    <span className="absolute top-5 left-[9px] h-full w-px bg-border" />
                  ) : null}
                  <span className="relative z-10 mt-0.5 flex size-5 shrink-0 items-center justify-center rounded-full border bg-card font-mono text-[10px]">
                    {i + 1}
                  </span>
                  <span className="flex flex-col">
                    <span className="text-sm">{item.step}</span>
                    <span className="text-xs text-muted-foreground">{item.note}</span>
                  </span>
                </li>
              ))}
            </ol>
          </CardContent>
        </Card>

        <div className="grid gap-4 sm:grid-cols-2">
          {MEMORY.map((fact) => (
            <div key={fact.title} className="flex flex-col gap-1.5">
              <h3 className="text-sm font-medium">{fact.title}</h3>
              <p className="text-sm leading-relaxed text-muted-foreground">{fact.body}</p>
            </div>
          ))}
        </div>
      </div>
    </Section>
  )
}
