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

  async generate(input: { prompt: string; orientation?: VideoOrientation }): Promise<GeneratedVideo> {
    const doFetch = this.options.fetch ?? fetch
    const sleep =
      this.options.sleep ?? ((ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms)))
    const seconds = this.options.seconds ?? 4
    const created = await doFetch(`${this.base}/videos`, {
      method: 'POST',
      headers: { ...this.headers, 'content-type': 'application/json' },
      body: JSON.stringify({
        model: this.options.model,
        prompt: input.prompt,
        seconds: String(seconds),
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
