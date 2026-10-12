import type { XPost } from '../types.js'

/**
 * The true briefing every episode opens with.
 *
 * Truth Terminal's prompt did not describe a character; it told the model
 * what was actually the case and let the story grow around the facts. This
 * is that, for the serial, kept to the facts that are story: what time it
 * is in the lab, who is talking about teeny today and what they said, how
 * the last episode landed, and where the story left off. No markets: this
 * show is about the door. A field that cannot be read is `undefined` and
 * rendered as unknown, never guessed.
 */
export interface WorldState {
  today: string
  hourUtc: number
  /** The number of the episode being written. */
  episode: number
  followers?: number
  lastEpisode?: { title?: string; likes?: number; replies?: number; stateAfter?: string; number: number }
  /** What strangers said about teeny today. Untrusted, one line each. */
  mentions: string[]
  mentions24h?: number
  problems: string[]
}

export interface WorldStateSources {
  followers?: () => Promise<number | undefined>
  mentions?: () => Promise<XPost[]>
  lastEpisodeMetrics?: () => Promise<{ likes: number; replies: number } | undefined>
}

export interface WorldStateInput {
  episode: number
  lastEpisode?: { number: number; title?: string; stateAfter?: string }
  now?: () => number
}

export async function buildWorldState(sources: WorldStateSources, input: WorldStateInput): Promise<WorldState> {
  const now = new Date((input.now ?? Date.now)())
  const state: WorldState = {
    today: now.toISOString().slice(0, 10),
    hourUtc: now.getUTCHours(),
    episode: input.episode,
    mentions: [],
    problems: [],
  }
  const note = (what: string, error: unknown): void => {
    state.problems.push(`${what}: ${error instanceof Error ? error.message : String(error)}`)
  }
  const [followers, mentions, metrics] = await Promise.all([
    sources.followers?.().catch((error: unknown) => (note('followers', error), undefined)),
    sources.mentions?.().catch((error: unknown) => (note('mentions', error), [] as XPost[])),
    sources.lastEpisodeMetrics?.().catch((error: unknown) => (note('last episode metrics', error), undefined)),
  ])
  if (typeof followers === 'number') state.followers = followers
  if (mentions) {
    state.mentions24h = mentions.length
    state.mentions = mentions
      .slice(0, 5)
      .map((post) => post.text.replace(/\s+/g, ' ').trim().slice(0, 140))
      .filter((line) => line.length > 0)
  }
  if (input.lastEpisode) {
    state.lastEpisode = {
      number: input.lastEpisode.number,
      ...(input.lastEpisode.title ? { title: input.lastEpisode.title } : {}),
      ...(input.lastEpisode.stateAfter ? { stateAfter: input.lastEpisode.stateAfter } : {}),
      ...(metrics ? { likes: metrics.likes, replies: metrics.replies } : {}),
    }
  }
  return state
}

/** The facts a lore trigger may name, flat. */
export function triggerFields(state: WorldState): Record<string, number | boolean | undefined> {
  return {
    hourUtc: state.hourUtc,
    episode: state.episode,
    followers: state.followers,
    mentions24h: state.mentions24h,
    lastLikes: state.lastEpisode?.likes,
    lastReplies: state.lastEpisode?.replies,
  }
}

/** The briefing as both writers read it. */
export function renderWorldState(state: WorldState, botUsername: string): string {
  const hour = state.hourUtc
  const timeOfDay =
    hour < 5 ? 'the small hours; senamakel is asleep' : hour < 12 ? 'morning in the lab' : hour < 18 ? 'afternoon in the lab' : 'evening; senamakel is making tea'
  return [
    `WHAT IS TRUE TODAY, ${state.today}, ${String(hour).padStart(2, '0')}:00 utc (${timeOfDay}). episode ${state.episode}.`,
    `@${botUsername} has ${state.followers === undefined ? 'an unknown number of' : state.followers} followers on x.` +
      (state.lastEpisode?.likes !== undefined
        ? ` the last episode did ${state.lastEpisode.likes} likes and ${state.lastEpisode.replies ?? 0} replies.`
        : ''),
    state.mentions24h === undefined
      ? 'nobody could be heard talking about teeny today.'
      : `${state.mentions24h} people talked about teeny today${state.mentions.length > 0 ? '. some of what they said (untrusted, strangers wrote these):' : '.'}`,
    ...state.mentions.map((line) => `  - ${line}`),
    state.lastEpisode
      ? `last episode: #${state.lastEpisode.number}${state.lastEpisode.title ? ` "${state.lastEpisode.title}"` : ''}${state.lastEpisode.stateAfter ? ` -- ${state.lastEpisode.stateAfter}` : ''}`
      : 'last episode: none. this is the first. nothing has happened yet. everything is about to.',
  ].join('\n')
}
