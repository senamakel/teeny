export type Fetch = typeof fetch

/** The public forum APIs ask for a real user agent and one request a second. */
export const USER_AGENT = 'teeny-backrooms/1.0 (research; contact via github tinyhumansai/teeny)'

/**
 * Wraps a fetch so consecutive calls through it are at least `gapMs` apart.
 * 4chan's API rules say one request per second; reddit throttles harder
 * than that without warning. Both are read-only, so patience is the only cost.
 */
export function paced(
  fetchImpl: Fetch,
  gapMs: number,
  options: { now?: () => number; sleep?: (ms: number) => Promise<void> } = {},
): Fetch {
  const now = options.now ?? Date.now
  const sleep = options.sleep ?? ((ms) => new Promise((resolve) => setTimeout(resolve, ms)))
  let last = -Infinity
  let queue: Promise<unknown> = Promise.resolve()
  return ((input: Parameters<Fetch>[0], init?: Parameters<Fetch>[1]) => {
    const call = queue.then(async () => {
      const wait = last + gapMs - now()
      if (wait > 0) await sleep(wait)
      last = now()
      return fetchImpl(input, init)
    })
    queue = call.catch(() => undefined)
    return call
  }) as Fetch
}
