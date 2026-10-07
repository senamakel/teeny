import type { ActivityLogger } from '../webhooks/intake.js'

/**
 * The X Activity API's other transport: `GET /2/activity/stream`, one long
 * HTTP response that carries every event the app's subscriptions match, as
 * newline-delimited JSON with a bare newline every 20 seconds to keep the
 * connection alive. The envelope is the same one the webhook POSTs,
 * `{ data: { event_type, payload, includes, ... } }`, so a reader hands each
 * line to the same intake.
 *
 * This is here because the webhook does not deliver `post.*` events any more
 * (see `ActivityIntake`), and the stream does. The SDK has a wrapper for the
 * endpoint but its reconnect is a stub that raises "not implemented", and a
 * reader that stays down after the first blip is worse than none, because
 * nothing would notice; so the endpoint is read by hand and reconnected with
 * a backoff X's own guidance describes: linear on a dropped connection,
 * exponential on an HTTP error, longest on a 429.
 *
 * One connection per app. X answers a second with 429, so this runs in the
 * ingest role only, next to the webhook and the Telegram poller.
 */
export class ActivityStream {
  private stopped = false
  private abort: AbortController | undefined
  private attempt = 0

  constructor(
    private readonly options: {
      bearerToken: string
      logger: ActivityLogger
      onEvent: (payload: Record<string, unknown>) => Promise<unknown>
      fetch?: typeof fetch
      url?: string
      /** Waits between attempts; injectable so a test does not sleep. */
      sleep?: (ms: number) => Promise<void>
      /** Give up after this many consecutive failures; 0 means never. */
      maxAttempts?: number
    },
  ) {}

  /** Connects and keeps reconnecting until `stop()`. Resolves when it stops. */
  async run(): Promise<void> {
    const { logger } = this.options
    const maxAttempts = this.options.maxAttempts ?? 0
    while (!this.stopped) {
      try {
        await this.connect()
        // A clean end of stream: X closes connections on its side from time
        // to time. That is a reconnect, not a failure, so the backoff resets.
        this.attempt = 0
        if (!this.stopped) logger.warn('activity stream ended; reconnecting')
      } catch (error) {
        if (this.stopped) break
        this.attempt += 1
        const delay = this.backoff(error)
        logger.error({ err: error, attempt: this.attempt, delayMs: delay }, 'activity stream failed')
        if (maxAttempts > 0 && this.attempt >= maxAttempts) {
          logger.error({ attempts: this.attempt }, 'activity stream gave up; the cron poll is all that reads X now')
          return
        }
        await (this.options.sleep ?? sleep)(delay)
      }
    }
  }

  stop(): void {
    this.stopped = true
    this.abort?.abort()
  }

  /**
   * How long to wait before the next attempt. X's streaming guidance: back
   * off linearly from a network drop, exponentially from an HTTP error, and
   * wait at least a minute on a 429, doubling from there.
   */
  private backoff(error: unknown): number {
    const status = error instanceof StreamHttpError ? error.status : undefined
    if (status === 429) return Math.min(60_000 * 2 ** (this.attempt - 1), 15 * 60_000)
    if (status !== undefined) return Math.min(5_000 * 2 ** (this.attempt - 1), 5 * 60_000)
    return Math.min(250 * this.attempt, 16_000)
  }

  private async connect(): Promise<void> {
    const { logger, onEvent } = this.options
    this.abort = new AbortController()
    const response = await (this.options.fetch ?? fetch)(
      this.options.url ?? 'https://api.x.com/2/activity/stream',
      {
        headers: { authorization: `Bearer ${this.options.bearerToken}` },
        signal: this.abort.signal,
      },
    )
    if (!response.ok) {
      const body = await response.text().catch(() => '')
      throw new StreamHttpError(response.status, body)
    }
    if (!response.body) throw new Error('the activity stream response has no body')

    logger.info('activity stream connected')
    this.attempt = 0
    const decoder = new TextDecoder()
    const reader = response.body.getReader()
    let buffer = ''
    try {
      for (;;) {
        const { done, value } = await reader.read()
        if (done) break
        buffer += decoder.decode(value, { stream: true })
        let newline = buffer.indexOf('\n')
        while (newline >= 0) {
          const line = buffer.slice(0, newline).trim()
          buffer = buffer.slice(newline + 1)
          newline = buffer.indexOf('\n')
          if (!line) continue // the keep-alive
          let parsed: unknown
          try {
            parsed = JSON.parse(line)
          } catch {
            logger.warn({ line: line.slice(0, 200) }, 'activity stream sent a line that is not JSON')
            continue
          }
          if (!parsed || typeof parsed !== 'object') continue
          const payload = parsed as Record<string, unknown>
          if (Array.isArray(payload.errors)) {
            // X reports an operational problem inline, in the stream, and
            // then usually closes it. Logged; the close is what reconnects.
            logger.warn({ errors: payload.errors }, 'activity stream reported errors')
            continue
          }
          if (!payload.data) continue
          await onEvent(payload).catch((error: unknown) => {
            logger.error({ err: error }, 'activity stream event was not handled')
          })
        }
      }
    } finally {
      reader.releaseLock()
    }
  }
}

export class StreamHttpError extends Error {
  constructor(
    readonly status: number,
    body: string,
  ) {
    super(`activity stream answered ${status}${body ? `: ${body.slice(0, 300)}` : ''}`)
    this.name = 'StreamHttpError'
  }
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}
