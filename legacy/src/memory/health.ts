/** What the store says about itself, reduced to what decides recall quality. */
export interface CortexHealth {
  reachable: boolean
  /** The store's own verdict: true when any check is failing. */
  degraded: boolean
  /** Whether embeddings work. When they do not, recall is keyword-only. */
  embeddings: { ok: boolean; okCount?: number; failCount?: number; lastError?: string }
  provider?: string
  error?: string
}

interface ReadyBody {
  degraded?: boolean
  embedding_provider?: string
  checks?: { embeddings?: { ok?: boolean; ok_count?: number; fail_count?: number; last_error?: string } }
}

/**
 * Reads the store's readiness report.
 *
 * Nothing on the turn's path notices a store that cannot embed: recall still
 * answers, from the keyword index alone, and the answers are merely worse.
 * From 2026-10-04 the store embedded nothing for a day because the router had
 * dropped the embedding ladder, and the only sign was replies that seemed to
 * have forgotten everyone. This is the sign, on a schedule, in the log.
 */
export async function checkCortex(
  apiUrl: string,
  options: { fetch?: typeof fetch; timeoutMs?: number } = {},
): Promise<CortexHealth> {
  const doFetch = options.fetch ?? fetch
  try {
    const response = await doFetch(new URL('/v1/admin/ready', apiUrl), {
      signal: AbortSignal.timeout(options.timeoutMs ?? 10_000),
    })
    const body = (await response.json().catch(() => ({}))) as ReadyBody
    const embeddings = body.checks?.embeddings
    return {
      reachable: true,
      degraded: body.degraded ?? !response.ok,
      embeddings: {
        ok: embeddings?.ok ?? response.ok,
        ...(embeddings?.ok_count === undefined ? {} : { okCount: embeddings.ok_count }),
        ...(embeddings?.fail_count === undefined ? {} : { failCount: embeddings.fail_count }),
        ...(embeddings?.last_error ? { lastError: embeddings.last_error.slice(0, 300) } : {}),
      },
      ...(body.embedding_provider ? { provider: body.embedding_provider } : {}),
    }
  } catch (error) {
    return {
      reachable: false,
      degraded: true,
      embeddings: { ok: false },
      error: error instanceof Error ? error.message : String(error),
    }
  }
}

/** Logs the report at the level it deserves; returns it for the caller. */
export async function reportCortexHealth(
  apiUrl: string,
  logger: { info: (object: unknown, message?: string) => void; warn: (object: unknown, message?: string) => void },
  options: { fetch?: typeof fetch } = {},
): Promise<CortexHealth> {
  const health = await checkCortex(apiUrl, options)
  if (!health.reachable) {
    logger.warn({ cortex: health }, 'memory: the store is unreachable; every recall this hour will come back empty')
  } else if (!health.embeddings.ok) {
    logger.warn({ cortex: health }, 'memory: the store cannot embed; recall is keyword-only until it can')
  } else if (health.degraded) {
    logger.warn({ cortex: health }, 'memory: the store reports itself degraded')
  } else {
    logger.info({ cortex: { provider: health.provider, embeddings: health.embeddings } }, 'memory: the store is healthy')
  }
  return health
}
