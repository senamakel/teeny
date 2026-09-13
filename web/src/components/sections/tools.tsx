import { Ban } from "lucide-react"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Section } from "@/components/section"
import { FORBIDDEN_TOOLS, TOOLS } from "@/lib/brain"

export function Tools() {
  const count = TOOLS.reduce((n, g) => n + g.tools.length, 0)
  return (
    <Section
      id="tools"
      eyebrow="Tools"
      title={`${count} things it can do. And the ones it cannot.`}
      lede="This is the whole list. A test asserts it exactly, so adding a capability is a deliberate act that fails the suite until someone decides."
    >
      <div className="grid gap-4 md:grid-cols-2">
        {TOOLS.map((group) => (
          <Card key={group.group} className="gap-3">
            <CardHeader>
              <CardTitle className="text-base">{group.group}</CardTitle>
              <CardDescription>{group.note}</CardDescription>
            </CardHeader>
            <CardContent>
              <ul className="flex flex-col divide-y">
                {group.tools.map((tool) => (
                  <li key={tool.name} className="flex flex-col gap-0.5 py-2 sm:flex-row sm:gap-3">
                    <code className="shrink-0 font-mono text-xs sm:w-40">{tool.name}</code>
                    <span className="text-sm text-muted-foreground">{tool.what}</span>
                  </li>
                ))}
              </ul>
            </CardContent>
          </Card>
        ))}

        <Card className="gap-3 border-destructive/30 md:col-span-2">
          <CardHeader>
            <CardTitle className="flex items-center gap-2 text-base">
              <Ban className="size-4 text-destructive" />
              Refused by the test suite
            </CardTitle>
            <CardDescription>
              A tool whose name contains any of these fails <code className="font-mono">test/capabilities.test.ts</code>.
              The account holds no keys, so &ldquo;send 5 SOL to this address&rdquo; has nothing to reach.
            </CardDescription>
          </CardHeader>
          <CardContent className="flex flex-wrap gap-1.5">
            {FORBIDDEN_TOOLS.map((name) => (
              <code
                key={name}
                className="rounded border border-destructive/30 bg-destructive/5 px-2 py-0.5 font-mono text-xs line-through decoration-destructive/60"
              >
                {name}
              </code>
            ))}
          </CardContent>
        </Card>
      </div>
    </Section>
  )
}
