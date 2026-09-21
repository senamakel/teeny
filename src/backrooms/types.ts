/**
 * The backrooms: a room where hot, tool-using agents talk for a while about
 * whatever came through the wire, each from a big and different context.
 *
 * Nothing here posts. A run is a record -- what the seed was, what each
 * agent was told, what was said, what was looked up -- kept so the material
 * can be read, scored, and compared before any of it is cut into content.
 */

export interface Seed {
  kind: 'news' | 'memory' | 'mention' | 'forum' | 'text'
  title?: string
  text: string
  /** Where it came from: a url, a handle, a board. */
  source?: string
}

export type ToolName = 'web_search' | 'recall_memory' | 'reddit_search' | 'fourchan_search' | 'x_search'

export interface AgentSpec {
  id: string
  /** The prompt marker its lines carry in the transcript, e.g. `teeny@tinyhumans:~/$`. */
  marker: string
  /** Its standing instructions; the assembled context is appended after. */
  system: string
  temperature: number
  tools: ToolName[]
  /** Whether VOICE.md is part of its context. Only teeny speaks in teeny's voice. */
  voice: boolean
}

export type BlockKind = 'seed' | 'tone' | 'engine' | 'voice' | 'lore' | 'past' | 'memory' | 'live' | 'forum' | 'tools'

export interface ContextBlock {
  kind: BlockKind
  label: string
  text: string
  tokens: number
}

export interface AgentContext {
  agent: AgentSpec
  blocks: ContextBlock[]
  tokens: number
  tone: string
  /** Sources that gave nothing, and why. Nothing is invented in their place. */
  problems: string[]
}

export interface ToolUse {
  name: string
  input: unknown
  summary: string
}

export interface BackroomsTurn {
  who: string
  text: string
  /** Set on the turn where the mood shifted, to what. */
  tone?: string
  tools: ToolUse[]
}

export interface BackroomsSettings {
  rng: number
  turns: number
  temps: Record<string, number>
  budget: number
  driftEvery: number
  maxToolCalls: number
}

export interface BackroomsRun {
  id: string
  startedAt: string
  finishedAt?: string
  seed: Seed
  agents: AgentContext[]
  turns: BackroomsTurn[]
  settings: BackroomsSettings
}

/** A reddit or 4chan post, whichever board or sub it came from. */
export interface ForumPost {
  site: 'reddit' | '4chan'
  /** `r/singularity`, `/x/`. */
  where: string
  title?: string
  text: string
  url: string
  /** Upvotes on reddit, replies on 4chan. */
  score?: number
  at?: string
}

export interface Forum {
  readonly site: ForumPost['site']
  search(query: string, options: { limit: number }): Promise<ForumPost[]>
}
