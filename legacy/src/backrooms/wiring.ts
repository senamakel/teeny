import type { LivePersona } from '../agent/soul.js'
import { existsSync, readFileSync } from 'node:fs'
import { createChatModel } from '../agent/graph.js'
import type { ExaSearch } from '../agent/tools/exa.js'
import type { Config } from '../config.js'
import type { InferenceLedger } from '../ledger/ledger.js'
import { OpenAITextModel } from '../media/reference.js'
import { ExaNewsSource, gatherNews } from '../morning/news.js'
import type { Memory, XPost } from '../types.js'
import { LangGraphBackroomsAgent } from './agent.js'
import { agentPresets, pickAgents } from './agents.js'
import { ContextBuilder } from './context.js'
import { distill, type Thought } from './distill.js'
import { gatherGmMaterial, gm as writeGm, type GmAgain, type GmPost } from './gm.js'
import { lore as writeLore, type LorePost } from './lore.js'
import { judge, stats, type Craziness, type RunStats } from './evaluate.js'
import { variants as makeVariants, type Variants } from './variants.js'
import { runBackrooms } from './room.js'
import { admissible, admissiblePost } from './screen.js'
import { FourchanForum } from './sources/fourchan.js'
import { paced, type Fetch } from './sources/http.js'
import { RedditForum } from './sources/reddit.js'
import { googleNews } from './sources/rss.js'
import { FileBackroomsStore, runId } from './store.js'
import { rng } from './tokens.js'
import { backroomsTools } from './tools.js'
import type { AgentContext, BackroomsRun, BackroomsTurn, Forum, Seed } from './types.js'

export interface BackroomsDeps {
  memory?: Memory
  x?: { searchPosts(query: string, options: { maxResults: number }): Promise<XPost[]> }
  exa?: ExaSearch
  fetch?: Fetch
  botUsername: string
  /** The rendered persona (soul, context, skills), for the good-morning layer. Absent, the voice file stands in. */
  persona?: LivePersona
  maxEmoji?: number
  ledger?: InferenceLedger
  logger?: { info(object: unknown, message?: string): void }
}

export interface RunOptions {
  agents?: string[]
  temps?: number[]
  turns?: number
  rng?: number
  budget?: number
  maxToolCalls?: number
  driftEvery?: number
  /** What tonight is about, one of the lore's engines; drawn at random when absent. */
  engine?: string
  score?: boolean
  /** Whether the transcript is read down to one thought at the end. On unless asked otherwise. */
  distill?: boolean
  /** Whether the thought is then written out as article, tweets, video and code. */
  variants?: boolean
  /** Whether the thought is written up as the good-morning post, with the soul, the lore and the last posts around it. */
  gm?: boolean
  onTurn?: (turn: BackroomsTurn, index: number) => void
}

export interface Backrooms {
  forums: Forum[]
  builder: ContextBuilder
  run(seed: Seed, options?: RunOptions): Promise<RunResult>
  /** The last layers on their own, for a run already on disk. */
  distill(run: BackroomsRun): Promise<Thought>
  variants(run: BackroomsRun, thought: Thought): Promise<Variants>
  /**
   * The good-morning post from a thought; `problems` collects the sources
   * that could not be read, `again` is the morning pipeline sending a draft
   * back (a repeat per memory, or the ranker's read).
   */
  gm(run: BackroomsRun, thought: Thought, problems?: string[], again?: GmAgain): Promise<GmPost>
  /** The afternoon lore post from a thought: the lab's page for the day, no good morning and no wire required. */
  lore(run: BackroomsRun, thought: Thought, problems?: string[]): Promise<LorePost>
}

export interface RunResult {
  run: BackroomsRun
  stats: RunStats
  judge?: Craziness
  thought?: Thought
  variants?: Variants
  gm?: GmPost
  /** Sources the good-morning layer could not read; the post was written without them. */
  problems: string[]
  saved: { json: string; markdown: string }
}

export function buildBackrooms(config: Config, deps: BackroomsDeps): Backrooms {
  const fetchImpl = paced(deps.fetch ?? fetch, 1_100)
  const forums: Forum[] = [
    new RedditForum({ fetch: fetchImpl, subs: config.REDDIT_SUBS, ...(deps.exa ? { exa: deps.exa } : {}) }),
    new FourchanForum({ fetch: fetchImpl, boards: config.FOURCHAN_BOARDS }),
  ]
  const builder = new ContextBuilder({
    ...(deps.memory ? { memory: deps.memory } : {}),
    ...(deps.x ? { x: deps.x } : {}),
    ...(deps.exa ? { exa: deps.exa } : {}),
    forums,
    voice: readFileSync(config.NARRATIVE_VOICE_PATH, 'utf8'),
    lore: readFileSync(config.NARRATIVE_LORE_PATH, 'utf8'),
    botUsername: deps.botUsername,
  })
  const presets = agentPresets({ botUsername: deps.botUsername })
  const store = new FileBackroomsStore(config.BACKROOMS_DIR)
  const text = (source: string, temperature: number): OpenAITextModel =>
    new OpenAITextModel({
      baseUrl: config.LLM_BASE_URL,
      apiKey: config.LLM_API_KEY,
      model: config.LLM_MODEL,
      source,
      temperature,
      ...(deps.ledger ? { ledger: deps.ledger } : {}),
    })
  const editorPrompt = existsSync(config.BACKROOMS_EDITOR_PATH) ? readFileSync(config.BACKROOMS_EDITOR_PATH, 'utf8').trim() : undefined
  const cold = text('backrooms:judge', 0.2)
  // Warmer than the judge: the thought is written, not classified.
  const editor = text('backrooms:editor', 0.5)
  const lore = readFileSync(config.NARRATIVE_LORE_PATH, 'utf8')
  const gmSources = {
    ...(deps.memory ? { memory: deps.memory } : {}),
    ...(deps.x ? { x: deps.x } : {}),
    persona: deps.persona ?? readFileSync(config.NARRATIVE_VOICE_PATH, 'utf8'),
    lore,
    botUsername: deps.botUsername,
    maxEmoji: deps.maxEmoji ?? config.MAX_EMOJI_PER_POST,
  }
  const writeMorning = async (run: BackroomsRun, thought: Thought, problems: string[] = [], again: GmAgain = {}): Promise<GmPost> =>
    writeGm(run, thought, await gatherGmMaterial(gmSources, problems), editor, again)
  const loreSources = {
    ...gmSources,
    historyQuery:
      'What lore posts, pages from the lab, has this account published recently, and what did each say? ' +
      'Also anything it has said this week about the lab, the tiny agents, its currency, the terminal, or senamakel.',
  }
  const writeAfternoon = async (run: BackroomsRun, thought: Thought, problems: string[] = []): Promise<LorePost> =>
    writeLore(run, thought, await gatherGmMaterial(loreSources, problems), editor)

  return {
    forums,
    builder,
    async run(seed, options = {}) {
      const specs = pickAgents(options.agents ?? config.BACKROOMS_AGENTS, presets)
      const temps: Record<string, number> = {}
      specs.forEach((spec, index) => {
        temps[spec.id] = options.temps?.[index] ?? spec.temperature
      })
      const settings = {
        rng: options.rng ?? Math.floor(Math.random() * 1_000_000),
        turns: options.turns ?? config.BACKROOMS_TURNS,
        temps,
        budget: options.budget ?? config.BACKROOMS_BUDGET_TOKENS,
        driftEvery: options.driftEvery ?? 4,
        maxToolCalls: options.maxToolCalls ?? config.BACKROOMS_TOOL_CALLS,
      }
      // The rng goes into the id: two rooms opened on the same seed in the
      // same minute would otherwise share a name and one would overwrite the other.
      const id = `${runId(new Date(), seed.title ?? seed.text.slice(0, 40))}-${settings.rng.toString(36)}`
      const random = rng(settings.rng)
      const contexts: AgentContext[] = []
      for (const spec of specs) {
        // One draw per agent, in order, so the same rng gives the same contexts.
        const draw = rng(Math.floor(random() * 2 ** 31))
        const context = await builder.build(spec, seed, draw, {
          budget: settings.budget,
          tools: settings.maxToolCalls > 0 ? spec.tools : [],
          ...(options.engine ? { engine: options.engine } : {}),
        })
        contexts.push(context)
        deps.logger?.info(
          { agent: spec.id, tokens: context.tokens, blocks: context.blocks.map((block) => `${block.kind}:${block.tokens}`), problems: context.problems },
          'backrooms context',
        )
      }
      const agents = specs.map((spec, index) => {
        const context = contexts[index]!
        return {
          spec,
          context,
          agent: new LangGraphBackroomsAgent({
            llm: createChatModel({
              baseUrl: config.LLM_BASE_URL,
              apiKey: config.LLM_API_KEY,
              model: config.LLM_MODEL,
              temperature: temps[spec.id] ?? spec.temperature,
              // A hot model that is allowed to run long runs into word salad; a turn is a few lines.
              maxTokens: 450,
              source: `backrooms:${spec.id}`,
              ...(deps.ledger ? { ledger: deps.ledger } : {}),
            }),
            context,
            tools:
              settings.maxToolCalls > 0
                ? backroomsTools(spec.tools, {
                    forums,
                    ...(deps.exa ? { exa: deps.exa } : {}),
                    ...(deps.memory ? { memory: deps.memory } : {}),
                    ...(deps.x ? { x: deps.x } : {}),
                  })
                : [],
            runId: id,
            maxToolCalls: settings.maxToolCalls,
          }),
        }
      })
      const run: BackroomsRun = { id, startedAt: new Date().toISOString(), seed, agents: contexts, turns: [], settings }
      run.turns = await runBackrooms({
        agents,
        seed,
        turns: settings.turns,
        random,
        driftEvery: settings.driftEvery,
        ...(options.onTurn ? { onTurn: options.onTurn } : {}),
      })
      run.finishedAt = new Date().toISOString()
      const counted = stats(run)
      let verdict: Craziness | undefined
      if (options.score) {
        try {
          verdict = await judge(run, cold)
        } catch (error) {
          deps.logger?.info({ error: error instanceof Error ? error.message : String(error) }, 'backrooms judge failed')
        }
      }
      let thought: Thought | undefined
      if (options.distill !== false) {
        try {
          thought = await distill(run, editor, editorPrompt)
        } catch (error) {
          deps.logger?.info({ error: error instanceof Error ? error.message : String(error) }, 'backrooms distill failed')
        }
      }
      let shapes: Variants | undefined
      if (thought && options.variants) {
        try {
          shapes = await makeVariants(run, thought, editor)
        } catch (error) {
          deps.logger?.info({ error: error instanceof Error ? error.message : String(error) }, 'backrooms variants failed')
        }
      }
      const problems: string[] = []
      let morning: GmPost | undefined
      if (thought && options.gm) {
        try {
          morning = await writeMorning(run, thought, problems)
        } catch (error) {
          deps.logger?.info({ error: error instanceof Error ? error.message : String(error) }, 'backrooms gm failed')
        }
      }
      const extras = {
        stats: counted,
        ...(verdict ? { judge: verdict } : {}),
        ...(thought ? { thought } : {}),
        ...(shapes ? { variants: shapes } : {}),
        ...(morning ? { gm: morning } : {}),
      }
      const saved = store.save(run, extras)
      return { run, ...extras, problems, saved }
    },
    distill: (run) => distill(run, editor, editorPrompt),
    variants: (run, thought) => makeVariants(run, thought, editor),
    gm: writeMorning,
    lore: writeAfternoon,
  }
}

// --- seeds -----------------------------------------------------------------

/** A story from Exa's news index, or from the Google News feed when Exa has nothing or is down. */
export async function seedFromNews(
  topic: string,
  options: { exa?: ExaSearch; fetch?: Fetch; random?: () => number } = {},
): Promise<Seed | undefined> {
  if (options.exa) {
    const { stories } = await gatherNews(new ExaNewsSource(options.exa), { topics: [topic], days: 2, perTopic: 6 })
    const story = stories.find((candidate) => admissible(`${candidate.title} ${candidate.excerpt}`))
    if (story) return { kind: 'news', title: story.title, text: story.excerpt, source: story.source }
  }
  const items = (await googleNews(topic, options.fetch ?? fetch)).filter((item) => admissible(`${item.title} ${item.description ?? ''}`))
  if (items.length === 0) return undefined
  const pick = items.slice(0, 10)
  const item = pick[Math.floor((options.random?.() ?? 0) * pick.length)]!
  return { kind: 'news', title: item.title, text: item.description?.trim() || item.title, ...(item.source ? { source: item.source } : {}) }
}

/**
 * A recall, read as prose: the memory engine's context block carries raw
 * captures (a morning post's whole JSON payload) alongside plain lines, and
 * a seed is something to riff on, not a record to parse.
 */
export async function seedFromMemory(memory: Memory, query: string): Promise<Seed | undefined> {
  const text = (await memory.recall(query))
    .split('\n')
    .map((line) => line.trim())
    .filter((line) => line && !line.startsWith('{') && !/^(GENERAL CONTEXT|ABOUT\b)/.test(line) && !/^-{3,}$/.test(line))
    .map((line) => line.replace(/^\[[^\]]*\]\s*(\[[^\]]*\]\s*)?/, ''))
    .filter((line) => line.length > 8)
    .join('\n')
    .trim()
  if (!text) return undefined
  return { kind: 'memory', title: query, text: text.slice(0, 1_200), source: 'memory' }
}

export async function seedFromMention(
  x: { searchPosts(query: string, options: { maxResults: number }): Promise<XPost[]> },
  botUsername: string,
): Promise<Seed | undefined> {
  const posts = await x.searchPosts(`(@${botUsername} OR to:${botUsername}) -from:${botUsername} -is:retweet`, { maxResults: 20 })
  const post = posts.find((candidate) => admissible(candidate.text))
  if (!post) return undefined
  return { kind: 'mention', text: post.text, source: post.authorUsername ? `@${post.authorUsername}` : 'x' }
}

export async function seedFromForum(forums: Forum[], query: string, random: () => number): Promise<Seed | undefined> {
  const posts = (await Promise.all(forums.map((forum) => forum.search(query, { limit: 6 }).catch(() => [])))).flat().filter(admissiblePost)
  if (posts.length === 0) return undefined
  const post = posts[Math.floor(random() * posts.length)]!
  return { kind: 'forum', ...(post.title ? { title: post.title } : {}), text: post.text, source: post.where }
}
