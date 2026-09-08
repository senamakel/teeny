/**
 * One JSON GET, with a deadline.
 *
 * The chart agent runs unattended once a day. A provider that accepts the
 * connection and then never answers would hang the run forever, so every call
 * carries a timeout rather than relying on the default, which is none.
 */
export async function fetchJson<T>(
  url: string,
  options: { timeoutMs?: number; headers?: Record<string, string> } = {},
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
      throw new Error(`${url} answered ${response.status}: ${body}`)
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
