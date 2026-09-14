import Link from "next/link"
import { ArrowRight } from "lucide-react"
import { AgentDesk } from "@/components/agent-desk"
import { RepoCard } from "@/components/sections/brain"
import { Button } from "@/components/ui/button"
import { Container } from "@/components/section"
import type { BrainSnapshot, RepoInfo } from "@/lib/company"

/** The brain on the front page: the desk, and the repository it commits to. */
export function HomeBrain({ snapshot, repo }: { snapshot: BrainSnapshot; repo: RepoInfo }) {
  return (
    <section className="border-b">
      <Container>
        <div className="mb-8 flex flex-wrap items-end justify-between gap-4">
          <div>
            <p className="mb-2 font-mono text-xs uppercase tracking-widest text-primary">The brain</p>
            <h2 className="text-2xl font-semibold tracking-tight sm:text-3xl">
              {snapshot.agents.length} agents that revise their own definition.
            </h2>
            <p className="mt-2 max-w-2xl text-muted-foreground">
              An OpenCompany company whose whole state is a public git repository. The scout looks, the scribe
              writes what it taught, the curator changes the definition, and every run is committed back.
            </p>
          </div>
          <Button variant="outline" render={<Link href="/brain" />}>
            Inside the brain
            <ArrowRight className="size-4" />
          </Button>
        </div>
        <AgentDesk snapshot={snapshot} />
        <div className="mt-6 grid gap-6 lg:grid-cols-2">
          <RepoCard repo={repo} compact />
          <ul className="grid content-start gap-3 sm:grid-cols-2">
            {snapshot.agents.map((agent) => (
              <li key={agent.id} className="rounded-xl border bg-card p-4">
                <p className="flex items-center justify-between text-sm font-medium">
                  {agent.role}
                  <code className="font-mono text-[11px] font-normal text-muted-foreground">{agent.id}</code>
                </p>
                <p className="mt-1 text-sm text-muted-foreground">{agent.description}</p>
              </li>
            ))}
          </ul>
        </div>
      </Container>
    </section>
  )
}
