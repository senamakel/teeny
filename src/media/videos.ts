import type { GeneratedVideo, VideoGenerator, VideoOrientation } from './types.js'

interface VideoJob {
  id?: string
  status?: 'queued' | 'in_progress' | 'completed' | 'failed' | string
  progress?: number
  seconds?: string | number
  error?: { message?: string; code?: string }
}

interface ErrorBody {
  error?: { message?: string }
}

const SORA_SECONDS = [4, 8, 12]

/**
 * Video generation through an OpenAI-shaped `/videos` endpoint (Sora, or
 * anything that speaks the same three routes): create a job, poll it, then
 * download the MP4. Rendering takes minutes, so the poll interval is coarse
 * and the deadline is the caller's business; a video that is not done by then
 * is abandoned, not posted late.
 */
export class OpenAIVideoGenerator implements VideoGenerator {
  constructor(
    private readonly options: {
      baseUrl: string
      apiKey: string
      model: string
      /** Clip length in seconds. Sora accepts 4, 8 or 12. */
      seconds?: number
      /** How long to wait for a render before giving up. */
      timeoutMs?: number
      pollMs?: number
      fetch?: typeof fetch
      sleep?: (ms: number) => Promise<void>
    },
  ) {}

  private get base(): string {
    return this.options.baseUrl.replace(/\/$/, '')
  }

  private get headers(): Record<string, string> {
    return { authorization: `Bearer ${this.options.apiKey}` }
  }

  async generate(input: {
    prompt: string
    orientation?: VideoOrientation
    seconds?: number
  }): Promise<GeneratedVideo> {
    const doFetch = this.options.fetch ?? fetch
    const sleep =
      this.options.sleep ?? ((ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms)))
    // Sora takes 4, 8 or 12 and nothing else; an ask in between rounds up.
    const wanted = input.seconds ?? this.options.seconds ?? 4
    const seconds = SORA_SECONDS.find((step) => step >= wanted) ?? 12
    const created = await doFetch(`${this.base}/videos`, {
      method: 'POST',
      headers: { ...this.headers, 'content-type': 'application/json' },
      body: JSON.stringify({
        model: this.options.model,
        prompt: input.prompt,
        seconds: String(seconds),
        // Sora has no square; a square ask gets the landscape frame.
        size: input.orientation === 'portrait' ? '720x1280' : '1280x720',
      }),
      signal: AbortSignal.timeout(60_000),
    })
    const job = (await created.json().catch(() => ({}))) as VideoJob & ErrorBody
    if (!created.ok || !job.id) {
      throw new Error(`video generation failed (${created.status}): ${job.error?.message ?? 'no detail'}`)
    }

    const deadline = Date.now() + (this.options.timeoutMs ?? 10 * 60_000)
    const poll = this.options.pollMs ?? 10_000
    let status = job.status ?? 'queued'
    let failure = job.error?.message
    while (status !== 'completed') {
      if (status === 'failed') {
        throw new Error(`video ${job.id} failed: ${failure ?? 'no detail'}`)
      }
      if (Date.now() >= deadline) {
        throw new Error(`video ${job.id} was still ${status} at the deadline`)
      }
      await sleep(poll)
      const check = await doFetch(`${this.base}/videos/${encodeURIComponent(job.id)}`, {
        headers: this.headers,
        signal: AbortSignal.timeout(30_000),
      })
      const next = (await check.json().catch(() => ({}))) as VideoJob & ErrorBody
      if (!check.ok) {
        throw new Error(`video status failed (${check.status}): ${next.error?.message ?? 'no detail'}`)
      }
      status = next.status ?? status
      failure = next.error?.message ?? failure
    }

    const content = await doFetch(`${this.base}/videos/${encodeURIComponent(job.id)}/content`, {
      headers: this.headers,
      signal: AbortSignal.timeout(120_000),
    })
    if (!content.ok) throw new Error(`video download failed (${content.status})`)
    const data = new Uint8Array(await content.arrayBuffer())
    if (data.byteLength === 0) throw new Error(`video ${job.id} downloaded empty`)
    return { data, mimeType: 'video/mp4', seconds }
  }
}

/**
 * The clip length, in every spelling the marketplace's sellers might read.
 *
 * Measured on 2026-09-14 against `kling-o3-standard-text-to-video`, every
 * job served by Venice: `seconds: "10"` alone, `duration: "10"` alone and
 * `duration: 10` alone each rendered the seller's own three seconds, while
 * a body carrying all five of these rendered ten. Surplus documents none of
 * them and the marketplace's credit ran out before the one that mattered
 * could be singled out, so the whole set goes, exactly as it was sent then.
 * Narrow it once there is credit to bisect with.
 */
function lengthFields(seconds: number): Record<string, unknown> {
  const text = String(seconds)
  return {
    seconds: text,
    duration: text,
    video_length: seconds,
    length: seconds,
    duration_seconds: seconds,
  }
}

const ASPECT: Record<VideoOrientation, string> = {
  landscape: '16:9',
  portrait: '9:16',
  square: '1:1',
}

const DONE = new Set(['completed', 'succeeded', 'success', 'done', 'finished', 'ready'])
const DEAD = new Set(['failed', 'error', 'canceled', 'cancelled', 'expired', 'rejected'])

/**
 * Video generation through the LLM ladder: `POST /video/generations` answers
 * 202 with a `media.job`, `GET /video/generations/{id}` is polled until it
 * finishes, and the clip is downloaded from wherever the finished job says
 * it is.
 *
 * The router relays the marketplace's job object untouched and documents no
 * finished shape, so the result is read leniently: any http(s) string under a
 * key ending in `url` that is not the job's own poll or cancel URL, an MP4
 * one preferred, or a base64 field. The error, when nothing fits, names the
 * keys that were there so the next person can add the right one.
 */
export class LadderVideoGenerator implements VideoGenerator {
  constructor(
    private readonly options: {
      baseUrl: string
      apiKey: string
      model: string
      timeoutMs?: number
      pollMs?: number
      fetch?: typeof fetch
      sleep?: (ms: number) => Promise<void>
    },
  ) {}

  private get base(): string {
    return this.options.baseUrl.replace(/\/$/, '')
  }

  private get headers(): Record<string, string> {
    return { authorization: `Bearer ${this.options.apiKey}` }
  }

  async generate(input: {
    prompt: string
    orientation?: VideoOrientation
    seconds?: number
  }): Promise<GeneratedVideo> {
    const doFetch = this.options.fetch ?? fetch
    const sleep =
      this.options.sleep ?? ((ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms)))
    const created = await doFetch(`${this.base}/video/generations`, {
      method: 'POST',
      headers: { ...this.headers, 'content-type': 'application/json' },
      body: JSON.stringify({
        model: this.options.model,
        prompt: input.prompt,
        aspect_ratio: ASPECT[input.orientation ?? 'square'],
        ...(input.seconds ? lengthFields(input.seconds) : {}),
      }),
      signal: AbortSignal.timeout(60_000),
    })
    let job = (await created.json().catch(() => ({}))) as Record<string, unknown> & ErrorBody
    const id = typeof job.id === 'string' ? job.id : undefined
    if (!created.ok || !id) {
      throw new Error(`video generation failed (${created.status}): ${job.error?.message ?? 'no detail'}`)
    }

    const deadline = Date.now() + (this.options.timeoutMs ?? 10 * 60_000)
    const poll = this.options.pollMs ?? 10_000
    for (;;) {
      const status = String(job.status ?? 'queued').toLowerCase()
      if (DONE.has(status)) break
      if (DEAD.has(status)) {
        throw new Error(`video ${id} ${status}: ${failureOf(job) ?? 'no detail'}`)
      }
      if (Date.now() >= deadline) throw new Error(`video ${id} was still ${status} at the deadline`)
      await sleep(poll)
      const check = await doFetch(`${this.base}/video/generations/${encodeURIComponent(id)}`, {
        headers: this.headers,
        signal: AbortSignal.timeout(30_000),
      })
      const next = (await check.json().catch(() => ({}))) as Record<string, unknown> & ErrorBody
      if (!check.ok) {
        throw new Error(`video status failed (${check.status}): ${next.error?.message ?? 'no detail'}`)
      }
      job = next
    }

    const inline = base64Of(job)
    if (inline) return { data: inline, mimeType: 'video/mp4' }
    const url = videoUrlOf(job)
    if (!url) {
      throw new Error(
        `video ${id} finished but nothing in the job looks like a clip; top-level keys: ${Object.keys(job).join(', ')}`,
      )
    }
    const content = await doFetch(url, { headers: this.headers, signal: AbortSignal.timeout(120_000) })
    if (!content.ok) throw new Error(`video download failed (${content.status})`)
    const data = new Uint8Array(await content.arrayBuffer())
    if (data.byteLength === 0) throw new Error(`video ${id} downloaded empty`)
    return { data, mimeType: 'video/mp4' }
  }
}

function failureOf(job: Record<string, unknown>): string | undefined {
  const error = job.error
  if (typeof error === 'string') return error
  if (error && typeof error === 'object' && typeof (error as { message?: unknown }).message === 'string') {
    return (error as { message: string }).message
  }
  return typeof job.failure_reason === 'string' ? job.failure_reason : undefined
}

/** Every `…url` string in the job, however deep, with the key path it sat under. */
function urlsIn(
  value: unknown,
  skip: Set<string>,
  path = '',
  found: Array<{ url: string; path: string }> = [],
): Array<{ url: string; path: string }> {
  if (Array.isArray(value)) {
    for (const entry of value) urlsIn(entry, skip, path, found)
  } else if (value && typeof value === 'object') {
    for (const [key, entry] of Object.entries(value as Record<string, unknown>)) {
      const here = path ? `${path}.${key}` : key
      if (typeof entry === 'string') {
        if (/url$/i.test(key) && /^https?:\/\//.test(entry) && !skip.has(entry)) {
          found.push({ url: entry, path: here.toLowerCase() })
        }
      } else {
        urlsIn(entry, skip, here, found)
      }
    }
  }
  return found
}

/**
 * The clip's URL, if the finished job has one: an `.mp4` first, then anything
 * filed under a key that says video, then anything not filed under a key
 * that says it is a still (thumbnail, preview, poster, image).
 */
export function videoUrlOf(job: Record<string, unknown>): string | undefined {
  const skip = new Set(
    [job.poll_url, job.cancel_url].filter((entry): entry is string => typeof entry === 'string'),
  )
  const urls = urlsIn(job, skip)
  const still = /thumb|preview|poster|image|cover/
  return (
    urls.find(({ url }) => /\.mp4(\?|$)/i.test(url)) ??
    urls.find(({ path }) => /video|clip/.test(path)) ??
    urls.find(({ path }) => !still.test(path)) ??
    urls[0]
  )?.url
}

function base64Of(job: Record<string, unknown>): Uint8Array | undefined {
  const candidates = [job.b64_json, job.video_base64, (job.output as { b64_json?: unknown } | undefined)?.b64_json]
  for (const candidate of candidates) {
    if (typeof candidate === 'string' && candidate.length > 0) {
      return new Uint8Array(Buffer.from(candidate, 'base64'))
    }
  }
  const data = job.data
  if (Array.isArray(data) && typeof (data[0] as { b64_json?: unknown } | undefined)?.b64_json === 'string') {
    return new Uint8Array(Buffer.from((data[0] as { b64_json: string }).b64_json, 'base64'))
  }
  return undefined
}
