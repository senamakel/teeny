import { ArrowUpRight, GitCommitHorizontal, Star } from "lucide-react"
import { PresenceIcon } from "@/components/icons/brand"
import { AgentDesk } from "@/components/agent-desk"
import { Badge } from "@/components/ui/badge"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Container } from "@/components/section"
import { OPENCOMPANY_URL, type BrainRun, type BrainSnapshot, type RepoInfo } from "@/lib/company"
import { compactTokens } from "@/lib/ledger"
import { cn } from "@/lib/utils"

function when(iso: string | undefined): string {
  if (!iso) return "—"
  const d = new Date(iso)
  return Number.isNaN(d.getTime()) ? iso : d.toISOString().slice(0, 16).replace("T", " ") + " UTC"
}

function Eyebrow({ children }: { children: React.ReactNode }) {
  return <p className="mb-2 font-mono text-xs uppercase tracking-widest text-primary">{children}</p>
}

/** The repository card: where the brain lives, and its last few commits. */
export function RepoCard({ repo, compact = false }: { repo: RepoInfo; compact?: boolean }) {
  return (
    <Card className="gap-4">
      <CardHeader>
        <CardTitle className="flex flex-wrap items-center gap-3 text-base">
          <span className="flex size-9 shrink-0 items-center justify-center rounded-lg border bg-muted/40">
            <PresenceIcon kind="github" className="size-4" />
          </span>
          <a href={repo.url} target="_blank" rel="noreferrer" className="font-mono hover:text-primary hover:underline underline-offset-4">
            {repo.name}
          </a>
          <span className="ml-auto flex items-center gap-3 font-mono text-xs text-muted-foreground">
            <span className="flex items-center gap-1"><Star className="size-3.5" />{repo.stars}</span>
            {repo.pushedAt ? <span>pushed {when(repo.pushedAt)}</span> : null}
          </span>
        </CardTitle>
        <CardDescription className="leading-relaxed">
          {repo.description ?? "The brain's public repository."} Run as an{" "}
          <a href={OPENCOMPANY_URL} target="_blank" rel="noreferrer" className="underline underline-offset-4 hover:text-foreground">OpenCompany</a>{" "}
          company: the checkout is the host&apos;s data root, the agents edit it, and a sync loop commits every run behind a secret scan.
        </CardDescription>
      </CardHeader>
      <CardContent>
        {repo.commits.length ? (
          <ol className="flex flex-col divide-y">
            {repo.commits.slice(0, compact ? 4 : 8).map((commit) => (
              <li key={commit.sha} className="flex items-start gap-3 py-2 text-sm">
                <GitCommitHorizontal className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
                <a href={commit.url} target="_blank" rel="noreferrer" className="min-w-0 flex-1 truncate hover:underline underline-offset-4">
                  {commit.message}
                </a>
                <span className="hidden shrink-0 font-mono text-xs text-muted-foreground sm:inline">{commit.author}</span>
                <span className="shrink-0 font-mono text-xs text-muted-foreground">{commit.sha.slice(0, 7)}</span>
              </li>
            ))}
          </ol>
        ) : (
          <p className="text-sm text-muted-foreground">
            {repo.live ? "No commits yet." : "GitHub could not be reached from here; the commits are on the repository page."}
          </p>
        )}
        <a href={`${repo.url}/commits/${repo.defaultBranch}`} target="_blank" rel="noreferrer" className="mt-3 inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground">
          every commit <ArrowUpRight className="size-3.5" />
        </a>
      </CardContent>
    </Card>
  )
}

const STATUS_STYLE: Record<string, string> = {
  succeeded: "border-emerald-400/40 text-emerald-400",
  running: "border-primary/40 text-primary",
  failed: "border-destructive/40 text-destructive",
  cancelled: "",
}

function RunRow({ run }: { run: BrainRun }) {
  const tokens = (run.usage?.input ?? 0) + (run.usage?.output ?? 0)
  const took = run.startedAtMillis && run.finishedAtMillis ? Math.round((run.finishedAtMillis - run.startedAtMillis) / 1000) : undefined
  return (
    <li className="grid grid-cols-[1fr_auto] items-center gap-x-4 gap-y-1 py-2 text-sm sm:grid-cols-[8rem_1fr_auto_auto_auto]">
      <span className="font-medium">{run.agentId}</span>
      <span className="font-mono text-xs text-muted-foreground sm:order-none order-last col-span-2 sm:col-span-1">
        {run.workflowRunId ? `${run.workflowRunId} · ${run.nodeId ?? ""}` : run.id}
      </span>
      <Badge variant="outline" className={cn("font-mono text-[10px]", STATUS_STYLE[run.status] ?? "")}>{run.status.replace("_", " ")}</Badge>
      <span className="font-mono text-xs text-muted-foreground">{tokens ? `${compactTokens(tokens)} tok` : ""}</span>
      <span className="font-mono text-xs text-muted-foreground">{took !== undefined ? `${took}s` : when(new Date(run.createdAtMillis).toISOString())}</span>
    </li>
  )
}

export function BrainSection({ snapshot, repo }: { snapshot: BrainSnapshot; repo: RepoInfo }) {
  const workflow = snapshot.workflows[0]
  return (
    <>
      <section className="border-b">
        <Container>
          <Eyebrow>Inside</Eyebrow>
          <h2 className="mb-2 text-2xl font-semibold tracking-tight sm:text-3xl">Watch them work.</h2>
          <p className="mb-8 max-w-2xl text-muted-foreground">
            {workflow?.description ?? "The agents, and how the work moves between them."}{" "}
            {snapshot.runs.length
              ? "This is the latest run, replayed from the host's own step record."
              : "No run is recorded yet; this rehearses the workflow from its definition."}
          </p>
          <AgentDesk snapshot={snapshot} />
        </Container>
      </section>

      <section className="border-b">
        <Container>
          <Eyebrow>The roster</Eyebrow>
          <h2 className="mb-8 text-2xl font-semibold tracking-tight sm:text-3xl">
            {snapshot.agents.length} agents, one repository.
          </h2>
          <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
            {snapshot.agents.map((agent) => (
              <Card key={agent.id} className="gap-3">
                <CardHeader>
                  <CardTitle className="flex items-center justify-between text-base">
                    {agent.role}
                    <code className="font-mono text-[11px] font-normal text-muted-foreground">{agent.id}</code>
                  </CardTitle>
                </CardHeader>
                <CardContent>
                  <CardDescription className="leading-relaxed">{agent.description}</CardDescription>
                  {agent.tier ? <Badge variant="outline" className="mt-3 font-mono text-[10px]">{agent.tier}</Badge> : null}
                </CardContent>
              </Card>
            ))}
          </div>
          {workflow ? (
            <div className="mt-10">
              <Eyebrow>The workflow</Eyebrow>
              <h3 className="mb-4 text-xl font-semibold tracking-tight">{workflow.name}</h3>
              <ol className="grid gap-3 md:grid-cols-2 lg:grid-cols-3">
                {workflow.nodes.map((node, index) => (
                  <li key={node.id} className="flex gap-3 rounded-xl border bg-card p-4">
                    <span className="font-mono text-xs text-muted-foreground">{String(index + 1).padStart(2, "0")}</span>
                    <div className="min-w-0">
                      <p className="flex flex-wrap items-center gap-2 text-sm font-medium">
                        {node.name}
                        <Badge variant="outline" className="font-mono text-[10px]">{node.kind}</Badge>
                        {node.agent ? <code className="font-mono text-[11px] text-primary">{node.agent}</code> : null}
                      </p>
                      {node.summary ? <p className="mt-1 text-sm text-muted-foreground">{node.summary}</p> : null}
                      {node.schedule ? <code className="mt-1 block font-mono text-[11px] text-muted-foreground">cron {node.schedule}</code> : null}
                    </div>
                  </li>
                ))}
              </ol>
            </div>
          ) : null}
        </Container>
      </section>

      <section className="border-b">
        <Container>
          <div className="grid gap-8 lg:grid-cols-2">
            <div>
              <Eyebrow>The repository</Eyebrow>
              <h2 className="mb-6 text-2xl font-semibold tracking-tight sm:text-3xl">Every run, committed.</h2>
              <RepoCard repo={repo} />
            </div>
            <div>
              <Eyebrow>The record</Eyebrow>
              <h2 className="mb-6 text-2xl font-semibold tracking-tight sm:text-3xl">Latest runs.</h2>
              <Card>
                <CardContent>
                  {snapshot.runs.length ? (
                    <ol className="divide-y">
                      {snapshot.runs.slice(0, 12).map((run) => (
                        <RunRow key={run.id} run={run} />
                      ))}
                    </ol>
                  ) : (
                    <p className="py-6 text-center text-sm text-muted-foreground">
                      No run recorded yet. The first one lands in <code className="font-mono">companies/{snapshot.company.slug}/runs.jsonl</code> and shows up here within a minute.
                    </p>
                  )}
                  {snapshot.source === "sample" ? (
                    <p className="mt-4 text-xs text-muted-foreground">
                      Sample run: the bot is not reachable from here. The roster and the workflow are the real ones from the repository.
                    </p>
                  ) : null}
                </CardContent>
              </Card>
            </div>
          </div>
        </Container>
      </section>
    </>
  )
}
