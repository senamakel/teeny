import { Lock } from "lucide-react"
import {
  Accordion,
  AccordionContent,
  AccordionItem,
  AccordionTrigger,
} from "@/components/ui/accordion"
import { Badge } from "@/components/ui/badge"
import { Section } from "@/components/section"
import { RULES, type Enforcement } from "@/lib/brain"

const ENFORCEMENT_NOTE: Record<Enforcement, string> = {
  "tool set": "held by what tools exist, not by asking the model",
  test: "a test fails if it changes",
  code: "applied in code after the model",
  prompt: "written into the system prompt",
}

export function Rules() {
  return (
    <Section
      id="rules"
      eyebrow="Rules"
      title="What no message can change"
      lede="A rule written in a prompt is a request. A rule enforced by the tool set or a test is a fact. Each rule here says which it is."
    >
      <div className="grid gap-8 lg:grid-cols-[1fr_1.6fr]">
        <div className="flex flex-col gap-3">
          <p className="text-sm text-muted-foreground">How a rule is held</p>
          {(Object.keys(ENFORCEMENT_NOTE) as Enforcement[]).map((key) => (
            <div key={key} className="flex items-center gap-3 text-sm">
              <Badge variant="outline" className="w-20 justify-center font-mono">
                {key}
              </Badge>
              <span className="text-muted-foreground">{ENFORCEMENT_NOTE[key]}</span>
            </div>
          ))}
        </div>

        <Accordion defaultValue={[RULES[0].title]}>
          {RULES.map((rule) => (
            <AccordionItem key={rule.title} value={rule.title}>
              <AccordionTrigger>
                <span className="flex flex-wrap items-center gap-2 pr-2 text-left">
                  {rule.unbreakable ? <Lock className="size-4 text-primary" /> : null}
                  <span>{rule.title}</span>
                  <span className="flex gap-1">
                    {rule.enforcement.map((e) => (
                      <Badge key={e} variant="secondary" className="font-mono">
                        {e}
                      </Badge>
                    ))}
                  </span>
                </span>
              </AccordionTrigger>
              <AccordionContent className="text-muted-foreground">{rule.body}</AccordionContent>
            </AccordionItem>
          ))}
        </Accordion>
      </div>
    </Section>
  )
}
