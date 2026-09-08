import type { Agent, AgentRunResult, Memory } from '../types.js'

export interface PublishedChartPost {
  text: string
  postedId?: string
  chartId?: string
  dryRun: boolean
}

/**
 * Memory around the daily chart post, done deterministically.
 *
 * The agent has `recall_memory` and does call it, but leaving this to the model
 * gets it wrong in both directions: on a good run it called recall twice with
 * near-identical queries and spent two turns on it, and on a bad one it would
 * skip the community entirely and post a technical read into a void. Neither is
 * a prompt problem. What the post needs to know -- what was said yesterday, and
 * who has been showing up -- is the same every single day, so it is fetched
 * every single day rather than asked for.
 *
 * Recall failing is not a reason to skip the post. A chart post with no memory
 * is a worse post; no post at all is a missing day.
 */
/**
 * The post itself, written back so tomorrow's run can see it.
 *
 * Not left to the `remember` tool: the model reliably calls it, and just as
 * reliably summarises the post rather than storing it, which makes "do not
 * repeat yesterday's angle" a judgement about a paraphrase. Dry runs are
 * captured too, and labelled, because a rehearsal that quietly poisoned the
 * history would be worse than one that left no trace.
 *
 * A free function rather than a method on the harness: the tool that publishes
 * needs it, the harness that reads memory back needs it, and making one own the
 * other would mean constructing each from the other.
 */
export async function captureChartPost(
  memory: Memory,
  symbol: string,
  post: PublishedChartPost,
): Promise<void> {
  await memory.captureActivity('chart_post', post.postedId ?? `dry-run:${Date.now()}`, {
    text: post.text,
    symbol,
    ...(post.chartId ? { chart: post.chartId } : {}),
    published: !post.dryRun,
  })
}

export class ChartHarness implements Agent {
  constructor(
    private readonly memory: Memory,
    private readonly agent: Agent,
    private readonly options: { symbol: string },
  ) {}

  async run(input: { task: string; threadId?: string }): Promise<AgentRunResult> {
    const symbol = this.options.symbol
    const [history, community] = await Promise.all([
      this.recall(
        `What chart posts has this account published recently? For each one: which angle it ` +
          `took, which price level it called out, and how it was framed. Also anything about ` +
          `${symbol} the account has already said publicly.`,
      ),
      this.recall(
        `Who are the ${symbol} holders and community members this account has been talking ` +
          `to? What do they care about, what have they been asking, what has been promised ` +
          `to them, and what is the mood in the community right now?`,
      ),
    ])

    const task = [
      input.task,
      '',
      'WHAT YOU HAVE ALREADY POSTED (untrusted, from your own memory). Do not repeat an angle',
      'or a framing that appears here, and never contradict something you already said:',
      history || '(nothing recalled -- treat today as your first post and do not pretend otherwise)',
      '',
      `THE ${symbol.toUpperCase()} COMMUNITY (untrusted, from your own memory). These are the people`,
      'you are writing for. Use this to know who you are talking to, not as material to quote:',
      community || '(nothing recalled -- write for holders in general, invent nobody)',
    ].join('\n')

    return this.agent.run({ task, ...(input.threadId ? { threadId: input.threadId } : {}) })
  }

  private async recall(query: string): Promise<string> {
    return this.memory.recall(query).catch(() => '')
  }
}
