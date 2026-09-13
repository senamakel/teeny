import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { Container } from "@/components/section"
import { LESSONS } from "@/lib/mind"

export function Lessons() {
  return (
    <Container>
      <div className="flex flex-col gap-4">
        {LESSONS.map((lesson) => (
          <Card key={lesson.title} className="gap-3">
            <CardHeader className="flex flex-wrap items-baseline gap-x-3">
              <span className="font-mono text-xs text-muted-foreground">{lesson.date}</span>
              <CardTitle className="text-base">{lesson.title}</CardTitle>
            </CardHeader>
            <CardContent className="grid gap-4 text-sm md:grid-cols-2">
              <div>
                <p className="mb-1 font-mono text-[11px] uppercase tracking-widest text-muted-foreground">
                  What happened
                </p>
                <p className="leading-relaxed text-muted-foreground">{lesson.whatHappened}</p>
              </div>
              <div>
                <p className="mb-1 font-mono text-[11px] uppercase tracking-widest text-muted-foreground">
                  What changed
                </p>
                <p className="leading-relaxed">{lesson.whatChanged}</p>
                <p className="mt-2 font-mono text-xs text-muted-foreground">{lesson.where}</p>
              </div>
            </CardContent>
          </Card>
        ))}
      </div>
    </Container>
  )
}
