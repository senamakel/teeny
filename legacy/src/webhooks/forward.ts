/**
 * A copy of every X delivery, sent on to another service.
 *
 * teeny is the one place X delivers to -- the app holds exactly one webhook
 * -- so anything else that wants the account's activity gets it from here.
 * The body goes out byte for byte with X's own signature header still on
 * it, so the receiver can verify it against the same consumer secret and
 * needs to trust nothing about the hop in between; a bearer of its own is
 * added when one is configured, for a receiver that would rather check that.
 *
 * Fire and forget, after teeny has already answered X: a delivery X does not
 * see acknowledged within seconds is retried and then dropped, and a slow
 * or absent receiver must never cost the account a mention. A copy that
 * does not arrive is logged and lost; the receiver has no replay.
 */
export interface WebhookForwarder {
  forward(body: Buffer, signature: string | undefined): Promise<void>
}

export interface ForwardOptions {
  url: string
  /** Sent as `Authorization: Bearer` when set. */
  token?: string
  /** How long one attempt may take. Defaults to 10 seconds. */
  timeoutMs?: number
  fetch?: typeof fetch
  logger?: {
    info: (object: unknown, message?: string) => void
    warn: (object: unknown, message?: string) => void
  }
}

export function buildWebhookForwarder(options: ForwardOptions): WebhookForwarder {
  const call = options.fetch ?? fetch
  const timeoutMs = options.timeoutMs ?? 10_000
  return {
    async forward(body, signature) {
      const headers: Record<string, string> = { 'content-type': 'application/json' }
      if (signature) headers['x-twitter-webhooks-signature'] = signature
      if (options.token) headers.authorization = `Bearer ${options.token}`
      try {
        const response = await call(options.url, {
          method: 'POST',
          headers,
          body: new Uint8Array(body),
          signal: AbortSignal.timeout(timeoutMs),
        })
        if (!response.ok) {
          options.logger?.warn({ url: options.url, status: response.status }, 'webhook copy refused')
          return
        }
        options.logger?.info({ url: options.url, bytes: body.byteLength }, 'webhook copy delivered')
      } catch (error) {
        options.logger?.warn(
          { url: options.url, err: error instanceof Error ? error.message : String(error) },
          'webhook copy not delivered',
        )
      }
    },
  }
}
