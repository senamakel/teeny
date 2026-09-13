import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Section } from "@/components/section"
import { PRINCIPLES } from "@/lib/brain"

export function Soul() {
  return (
    <Section
      id="soul"
      eyebrow="Soul"
      title="Who it is, in its own file"
      lede="The persona lives in SOUL.md, in the repo, where anyone can read the exact words the model is given. This is the short version."
    >
      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
        {PRINCIPLES.map((p) => (
          <Card key={p.title} className="gap-2">
            <CardHeader>
              <CardTitle className="text-base">{p.title}</CardTitle>
            </CardHeader>
            <CardContent>
              <CardDescription className="leading-relaxed">{p.body}</CardDescription>
            </CardContent>
          </Card>
        ))}
      </div>
      <blockquote className="mt-8 max-w-2xl border-l-2 border-primary pl-4 text-muted-foreground">
        &ldquo;gm tiny humans. honestly just thinking about how big things are really just a
        bunch of tiny steps stacked together. what are we building today?&rdquo;
        <footer className="mt-2 font-mono text-xs">— the example post in SOUL.md</footer>
      </blockquote>
    </Section>
  )
}
