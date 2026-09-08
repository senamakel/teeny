/**
 * Serialises calls to one provider and keeps a minimum gap between them.
 *
 * GeckoTerminal's free tier is roughly thirty calls a minute, shared across
 * everything on the box. A single chart run reads four timeframes, holders,
 * flow and the raw candles, and firing those in parallel -- which is what
 * `Promise.all` in the timeframes tool does -- reliably produced a burst of
 * 429s. The agent then wrote a confident post about a chart it had not been
 * able to read, which is a worse failure than a slow run.
 *
 * A queue rather than a token bucket because the ordering is free: nothing here
 * is latency-sensitive, the job has a whole day, and a queue cannot burst by
 * construction.
 */
export function createLimiter(minIntervalMs: number): <T>(work: () => Promise<T>) => Promise<T> {
  let tail: Promise<unknown> = Promise.resolve()
  let last = 0

  return <T>(work: () => Promise<T>): Promise<T> => {
    const run = tail.then(async () => {
      const wait = last + minIntervalMs - Date.now()
      if (wait > 0) await new Promise((resolve) => setTimeout(resolve, wait))
      try {
        return await work()
      } finally {
        // Stamped after the call, not before, so a slow request does not have
        // its own duration counted as part of the gap that follows it.
        last = Date.now()
      }
    })
    // The chain must survive a failed call, or one rejection wedges every
    // request queued behind it.
    tail = run.catch(() => undefined)
    return run
  }
}
