import type { MetricSample, OwnPost, PostLedger } from './ledger.js'

export interface PostMetrics {
  id: string
  impressions: number
  likes: number
  replies: number
  reposts: number
  quotes: number
}

export interface HeatJobOptions {
  ledger: PostLedger
  x: { postMetrics(ids: string[]): Promise<PostMetrics[]> }
  /** How far back a post is still watched. */
  lookbackMs: number
  /** One slot of the cadence: how long a hold lasts per extension. */
  gapMs: number
  /** Below this many impressions an hour nothing is hot, whatever the baseline. */
  impressionsPerHour: number
  /** Hot also means this many times the account's usual pace. */
  baselineMultiple: number
  /** How many slots a hold may grow to, counted from when the post first ran hot. */
  maxHoldSlots: number
  now?: () => number
  logger?: { info: (object: unknown, message?: string) => void; warn: (object: unknown, message?: string) => void }
}

export interface HeatReading {
  postId: string
  impressions: number
  velocity?: number
  baseline?: number
  hot: boolean
  holdUntil?: string
}

/**
 * Watches the account's recent posts and notices one taking off.
 *
 * Every run reads the counters of everything posted in the last day and
 * keeps the reading. Two readings give a velocity, impressions an hour, and
 * a post is hot when that velocity is both large in absolute terms and a
 * multiple of what the account's posts usually do -- one number alone
 * misreads a big account's ordinary day or a small account's good hour.
 *
 * A hot post gets a hold on the timeline: one slot from now, and if it is
 * still hot at the next reading, extended, up to `maxHoldSlots` from when
 * it first ran hot. The cadence refuses own posts while the hold runs, and
 * the reply budget hands the held slots' replies to the hot post's thread.
 */
export class HeatJob {
  private running = false

  constructor(private readonly options: HeatJobOptions) {}

  async run(): Promise<HeatReading[]> {
    if (this.running) return []
    this.running = true
    try {
      const now = (this.options.now ?? Date.now)()
      const nowIso = new Date(now).toISOString()
      const posts = await this.options.ledger.since(new Date(now - this.options.lookbackMs).toISOString())
      if (posts.length === 0) return []

      const metrics = await this.options.x.postMetrics(posts.map((post) => post.postId))
      const readings: HeatReading[] = []
      for (const post of posts) {
        const current = metrics.find((entry) => entry.id === post.postId)
        if (!current) continue
        const sample: MetricSample = {
          at: nowIso,
          impressions: current.impressions,
          likes: current.likes,
          replies: current.replies,
          reposts: current.reposts,
          quotes: current.quotes,
        }
        const velocity = velocityOf(post, sample)
        await this.options.ledger.sample(post.postId, sample)
        const baseline = baselineOf(posts.filter((other) => other.postId !== post.postId))
        const hot =
          velocity !== undefined &&
          velocity >= this.options.impressionsPerHour &&
          (baseline === undefined || velocity >= baseline * this.options.baselineMultiple)

        let holdUntil: string | undefined
        if (hot) {
          const since = post.hot?.since ?? nowIso
          const cap = Date.parse(since) + this.options.maxHoldSlots * this.options.gapMs
          const extended = Math.min(now + this.options.gapMs, cap)
          const current = post.hot ? Date.parse(post.hot.holdUntil) : 0
          holdUntil = new Date(Math.max(current, extended)).toISOString()
          if (holdUntil !== post.hot?.holdUntil) {
            await this.options.ledger.setHot(post.postId, { since, holdUntil })
            this.options.logger?.info(
              { postId: post.postId, velocity, baseline, impressions: sample.impressions, holdUntil },
              'heat: the post is hot; holding the timeline for it',
            )
          }
        }
        readings.push({
          postId: post.postId,
          impressions: sample.impressions,
          ...(velocity === undefined ? {} : { velocity }),
          ...(baseline === undefined ? {} : { baseline }),
          hot,
          ...(holdUntil ? { holdUntil } : {}),
        })
      }
      return readings
    } finally {
      this.running = false
    }
  }
}

/** Impressions an hour between the last kept reading and this one. */
export function velocityOf(post: OwnPost, sample: MetricSample): number | undefined {
  const previous = post.samples[post.samples.length - 1]
  const from = previous ?? { at: post.postedAt, impressions: 0 }
  const hours = (Date.parse(sample.at) - Date.parse(from.at)) / 3_600_000
  if (hours <= 0) return undefined
  return Math.max(0, sample.impressions - from.impressions) / hours
}

/**
 * What the account's other recent posts did at their best: the median of
 * each post's peak velocity. Undefined until two posts have readings, in
 * which case only the absolute threshold applies.
 */
export function baselineOf(others: OwnPost[]): number | undefined {
  const peaks = others
    .map((post) => {
      let peak = 0
      let from: { at: string; impressions: number } = { at: post.postedAt, impressions: 0 }
      for (const sample of post.samples) {
        const hours = (Date.parse(sample.at) - Date.parse(from.at)) / 3_600_000
        if (hours > 0) peak = Math.max(peak, (sample.impressions - from.impressions) / hours)
        from = sample
      }
      return post.samples.length > 0 ? peak : undefined
    })
    .filter((peak): peak is number => peak !== undefined)
    .sort((a, b) => a - b)
  if (peaks.length < 2) return undefined
  const middle = Math.floor(peaks.length / 2)
  return peaks.length % 2 === 0 ? (peaks[middle - 1]! + peaks[middle]!) / 2 : peaks[middle]!
}
