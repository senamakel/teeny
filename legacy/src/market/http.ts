/**
 * One JSON GET, with a deadline.
 *
 * The chart agent runs unattended once a day. A provider that accepts the
 * connection and then never answers would hang the run forever, so every call
 * carries a timeout rather than relying on the default, which is none.
 */
export async function fetchJson<T>(
  url: string,
  options: {
    timeoutMs?: number
    headers?: Record<string, string>
    /** How many times to retry a 429 or a 5xx. Defaults to two. */
    retries?: number
    /** Injected in tests so a backoff does not make the suite wait. */
    sleep?: (ms: number) => Promise<void>
  } = {},
): Promise<T> {
  const retries = options.retries ?? 2
  const sleep = options.sleep ?? ((ms: number) => new Promise((resolve) => setTimeout(resolve, ms)))
  let last: Error | undefined

  for (let attempt = 0; attempt <= retries; attempt += 1) {
    try {
      return await once<T>(url, options)
    } catch (error) {
      last = error as Error
      // A rate limit and a bad gateway are worth waiting out; a 404 is not, and
      // neither is a malformed request. Retrying those just spends the budget
      // the retryable ones need.
      if (!isWorthRetrying(last) || attempt === retries) throw last
      // When the server said how long to wait, wait that long. Backing off on
      // our own schedule against an explicit Retry-After just burns another
      // attempt on a request it has already told us it will refuse.
      const asked = retryAfterMs(last)
      await sleep(asked ?? 500 * 2 ** attempt)
    }
  }
  throw last ?? new Error(`${url} failed`)
}

/**
 * The free tiers behind this service are strict and shared: GeckoTerminal
 * answered 429 partway through the first run that read four timeframes, and the
 * post silently lost its holder data because a rate limit looked exactly like a
 * missing feature.
 */
/** `Retry-After` in seconds, when the response carried one. */
function retryAfterMs(error: Error): number | undefined {
  const match = /retry-after:\s*(\d+)/i.exec(error.message)
  const seconds = match?.[1] === undefined ? undefined : Number(match[1])
  return seconds === undefined || !Number.isFinite(seconds)
    ? undefined
    : Math.min(seconds, 30) * 1_000
}

function isWorthRetrying(error: Error): boolean {
  return /\b(429|500|502|503|504)\b/.test(error.message) || error.name === 'AbortError'
}

async function once<T>(
  url: string,
  options: { timeoutMs?: number; headers?: Record<string, string> },
): Promise<T> {
  const controller = new AbortController()
  const timer = setTimeout(() => controller.abort(), options.timeoutMs ?? 15_000)
  try {
    const response = await fetch(url, {
      signal: controller.signal,
      headers: {
        accept: 'application/json',
        // Several of these providers sit behind a CDN that refuses a request
        // with no user agent outright.
        'user-agent': 'teeny-chart/1.0 (+https://tinyhumans.ai)',
        ...options.headers,
      },
    })
    if (!response.ok) {
      const body = (await response.text().catch(() => '')).slice(0, 300)
      const after = response.headers.get('retry-after')
      throw new Error(
        `${url} answered ${response.status}: ${body}` +
          (after ? ` (retry-after: ${after})` : ''),
      )
    }
    return (await response.json()) as T
  } finally {
    clearTimeout(timer)
  }
}

/** A number from an API that may spell it as a string, or not send it at all. */
export function num(value: unknown): number | undefined {
  if (typeof value === 'number') return Number.isFinite(value) ? value : undefined
  if (typeof value === 'string' && value.trim()) {
    const parsed = Number(value)
    return Number.isFinite(parsed) ? parsed : undefined
  }
  return undefined
}
